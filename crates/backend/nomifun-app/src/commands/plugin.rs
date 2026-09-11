//! Small product-facing headless clients for the Plugin and MiniApp APIs.
//!
//! This module deliberately stays an HTTP adapter. The desktop/server process
//! remains the sole owner of AppServices, SQLite, Runtime, Host, and lifecycle
//! state; the CLI only builds typed requests against those existing routes.

use std::process::ExitCode;

use nomifun_api_types::{
    ApiResponse, ApplyPluginCandidateRequest, ApplyPluginTargetDto,
    BuildPluginProjectRequest, ErrorResponse, MiniAppLibraryResponseDto,
    DeletePluginDataRequest, MiniAppWorkshopDto, PluginDetailDto,
    PluginLibraryResponseDto, PluginProjectDetailDto, RetryPluginRequest,
    RestorePluginPreviousRequest, SetPluginAutoApplyRequest, SetPluginEnabledRequest,
    SharePluginRequest, TestPluginCandidateRequest, UninstallPluginRequest,
};
use reqwest::{Client, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cli::{
    Cli, HeadlessConnectionArgs, MiniAppCommand, MiniAppListArgs,
    MiniAppShowArgs, PluginCandidateApplyArgs, PluginCandidateCommand,
    PluginCandidateDiscardArgs,
    PluginAutoApplyCommand, PluginCandidateShowArgs, PluginCommand, PluginImportArgs,
    PluginListArgs, PluginMountArgs, PluginMountCommand, PluginProjectArgs,
    PluginProjectCommand, PluginShareCommand, PluginShareExportArgs, PluginShowArgs,
    PluginTestArgs,
};

const DEFAULT_URL: &str = "http://127.0.0.1:25808";
const PLUGIN_PLATFORM_DIRECTORY: &str = "plugin-platform";
const PLUGIN_AUTHORING_DIRECTORY: &str = "authoring";

/// Execute one headless Plugin command and map the result to the documented
/// `0/1/2` process contract.
pub async fn run_plugin(cli: &Cli, operation: &PluginCommand) -> ExitCode {
    finish(run_plugin_inner(cli, operation).await)
}

/// Execute one headless MiniApp command and map the result to the documented
/// `0/1/2` process contract.
pub async fn run_miniapp(operation: &MiniAppCommand) -> ExitCode {
    finish(run_miniapp_inner(operation).await)
}

async fn run_plugin_inner(
    cli: &Cli,
    operation: &PluginCommand,
) -> Result<Value, CliFailure> {
    match operation {
        PluginCommand::List(args) => run_plugin_list(args).await,
        PluginCommand::Show(args) => run_plugin_show(args).await,
        PluginCommand::Project { operation } => match operation {
            PluginProjectCommand::SourcePath(args) => {
                run_source_path(cli, args).await
            }
        },
        PluginCommand::Build(args) => run_build(args).await,
        PluginCommand::Test(args) => run_test(args).await,
        PluginCommand::Import(args) => run_import(args).await,
        PluginCommand::Share { operation } => match operation {
            PluginShareCommand::Export(args) => run_share_export(args).await,
        },
        PluginCommand::AutoApply { operation } => match operation {
            PluginAutoApplyCommand::Enable(args) => run_auto_apply(args, true, false).await,
            PluginAutoApplyCommand::Disable(args) => run_auto_apply(args, false, false).await,
            PluginAutoApplyCommand::Retry(args) => run_auto_apply(args, true, true).await,
        },
        PluginCommand::Candidate { operation } => match operation {
            PluginCandidateCommand::Show(args) => run_candidate_show(args).await,
            PluginCandidateCommand::Discard(args) => run_candidate_discard(args).await,
            PluginCandidateCommand::Apply(args) => run_apply(args).await,
            PluginCandidateCommand::Restore(args) => run_restore(args).await,
        },
        PluginCommand::Mount { operation } => match operation {
            PluginMountCommand::Enable(args) => run_set_enabled(args, true).await,
            PluginMountCommand::Disable(args) => run_set_enabled(args, false).await,
            PluginMountCommand::Retry(args) => run_retry_mount(args).await,
            PluginMountCommand::Uninstall(args) => run_uninstall_mount(args).await,
            PluginMountCommand::DeleteData(args) => run_delete_mount_data(args).await,
        },
    }
}

async fn run_miniapp_inner(
    operation: &MiniAppCommand,
) -> Result<Value, CliFailure> {
    match operation {
        MiniAppCommand::List(args) => run_miniapp_list(args).await,
        MiniAppCommand::Show(args) => run_miniapp_show(args).await,
    }
}

async fn run_plugin_list(args: &PluginListArgs) -> Result<Value, CliFailure> {
    let client = HeadlessClient::new(&args.connection)?;
    let response: ApiResponse<PluginLibraryResponseDto> =
        client.get_api("/api/plugins").await?;
    to_value(response)
}

async fn run_plugin_show(args: &PluginShowArgs) -> Result<Value, CliFailure> {
    let mount_id = checked_segment(&args.mount_id, "mount_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let response = client
        .get_api::<PluginDetailDto>(&format!("/api/plugin-mounts/{mount_id}"))
        .await?;
    to_value(response)
}

async fn run_candidate_show(
    args: &PluginCandidateShowArgs,
) -> Result<Value, CliFailure> {
    let project_id = checked_segment(&args.project_id, "project_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let response = client
        .get_api::<PluginProjectDetailDto>(&format!(
            "/api/plugin-projects/{project_id}"
        ))
        .await?;
    to_value(response)
}

async fn run_candidate_discard(
    args: &PluginCandidateDiscardArgs,
) -> Result<Value, CliFailure> {
    let project_id = checked_segment(&args.project_id, "project_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let project = fetch_project(&client, &project_id).await?;
    let ready = project.ready.clone().ok_or_else(|| {
        CliFailure::state("the Plugin Project has no Ready Candidate")
    })?;
    let request = nomifun_api_types::DiscardPluginCandidateRequest {
        project_id: project.summary.project_id.clone(),
        expected_project_revision: project.summary.project_revision,
        expected_build_generation: project.summary.build_generation,
        candidate_id: ready.candidate.candidate_id,
        expected_candidate_digest: ready.candidate.candidate_digest,
    };
    let response: ApiResponse<PluginProjectDetailDto> = client
        .post_api(
            &format!("/api/plugin-projects/{project_id}/candidate/discard"),
            &request,
        )
        .await?;
    to_value(response)
}

async fn run_source_path(
    cli: &Cli,
    args: &PluginProjectArgs,
) -> Result<Value, CliFailure> {
    let project_id = checked_segment(&args.project_id, "project_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let project = fetch_project(&client, &project_id).await?;

    let (managed_relative_path, source_path) =
        if !matches!(
            project.summary.source_state,
            nomifun_api_types::PluginProjectSourceStateDto::RuntimeOnly
        ) {
            let owner_id = fetch_owner_id(&client).await?;
            let owner_id = checked_segment(&owner_id, "owner_id")?;
            let relative = format!(
                "sources/{owner_id}/projects/{project_id}/source"
            );
            let absolute = cli
                .data_dir
                .join(PLUGIN_PLATFORM_DIRECTORY)
                .join(PLUGIN_AUTHORING_DIRECTORY)
                .join("sources")
                .join(&owner_id)
                .join("projects")
                .join(&project_id)
                .join("source");
            (Some(relative), Some(absolute.display().to_string()))
        } else {
            (None, None)
        };

    let output = SourcePathOutput {
        project_id,
        source_state: project.summary.source_state,
        source_snapshot_digest: project.source_snapshot_digest,
        dependency_lock_digest: project.dependency_lock_digest,
        managed_relative_path,
        source_path,
    };
    to_value(ApiResponse::ok(output))
}

async fn run_build(args: &PluginProjectArgs) -> Result<Value, CliFailure> {
    let project_id = checked_segment(&args.project_id, "project_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let project = fetch_project(&client, &project_id).await?;
    let source_digest = project
        .source_snapshot_digest
        .clone()
        .ok_or_else(|| {
            CliFailure::state(
                "the Plugin Project has no editable Source snapshot",
            )
        })?;
    let lock_digest = project
        .dependency_lock_digest
        .clone()
        .ok_or_else(|| {
            CliFailure::state(
                "the Plugin Project has no exact dependency lock",
            )
        })?;

    let request = BuildPluginProjectRequest {
        project_id: project.summary.project_id.clone(),
        expected_project_revision: project.summary.project_revision,
        expected_build_generation: project.summary.build_generation,
        expected_source_snapshot_digest: source_digest,
        expected_dependency_lock_digest: lock_digest,
    };
    let response: ApiResponse<PluginProjectDetailDto> = client
        .post_api(
            &format!("/api/plugin-projects/{project_id}/build"),
            &request,
        )
        .await?;
    to_value(response)
}

async fn run_test(args: &PluginTestArgs) -> Result<Value, CliFailure> {
    validate_digest(
        &args.resolved_test_input_digest,
        "resolved-test-input-digest",
    )?;
    let project_id = checked_segment(&args.project_id, "project_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let project = fetch_project(&client, &project_id).await?;
    let ready = project.ready.clone().ok_or_else(|| {
        CliFailure::state("the Plugin Project has no Ready Candidate")
    })?;

    let (expected_config_revision, expected_credential_bindings_revision) =
        match project.summary.linked_mount_id.as_deref() {
            Some(mount_id) => {
                let mount_id = checked_segment(mount_id, "mount_id")?;
                let mount = fetch_mount(&client, &mount_id).await?;
                (
                    mount.config.config_revision,
                    mount.credential_bindings_revision,
                )
            }
            None => (0, 0),
        };

    let request = TestPluginCandidateRequest {
        project_id: project.summary.project_id.clone(),
        expected_project_revision: project.summary.project_revision,
        expected_build_generation: project.summary.build_generation,
        candidate_id: ready.candidate.candidate_id,
        expected_candidate_digest: ready.candidate.candidate_digest,
        expected_config_revision,
        expected_credential_bindings_revision,
        resolved_test_input_digest: args.resolved_test_input_digest.clone(),
    };
    let response: ApiResponse<PluginProjectDetailDto> = client
        .post_api(
            &format!("/api/plugin-projects/{project_id}/test"),
            &request,
        )
        .await?;
    to_value(response)
}

async fn run_import(args: &PluginImportArgs) -> Result<Value, CliFailure> {
    validate_digest(&args.expected_digest, "expected-digest")?;
    let client = HeadlessClient::new(&args.connection)?;
    let library: ApiResponse<PluginLibraryResponseDto> = client.get_api("/api/plugins").await?;
    let library = require_data(library, "Plugin library")?;
    let request = nomifun_api_types::ImportPluginRequest {
        expected_library_revision: library.library_revision,
        import_kind: if args.share_bundle {
            nomifun_api_types::PluginImportKindDto::ShareBundle
        } else {
            nomifun_api_types::PluginImportKindDto::PrebuiltArtifact
        },
        source_path: args.source_path.display().to_string(),
        expected_bundle_or_artifact_digest: args.expected_digest.to_ascii_lowercase(),
        target_project_id: None,
        expected_project_revision: None,
    };
    let response: ApiResponse<PluginProjectDetailDto> =
        client.post_api("/api/plugin-imports", &request).await?;
    to_value(response)
}

async fn run_share_export(args: &PluginShareExportArgs) -> Result<Value, CliFailure> {
    let project_id = checked_segment(&args.project_id, "project_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let project = fetch_project(&client, &project_id).await?;
    let request = if args.current_mount {
        if args.include_source {
            return Err(CliFailure::usage(
                "--include-source is only valid for an exact Ready Candidate",
            ));
        }
        let mount_id = project.summary.linked_mount_id.as_deref().ok_or_else(|| {
            CliFailure::state("the Plugin Project has no linked Mount")
        })?;
        let mount_id = checked_segment(mount_id, "mount_id")?;
        let mount = fetch_mount(&client, &mount_id).await?;
        let current = mount.summary.current.ok_or_else(|| {
            CliFailure::state("the linked Plugin Mount has no current target")
        })?;
        SharePluginRequest {
            project_id: project.summary.project_id.clone(),
            expected_project_revision: project.summary.project_revision,
            source: nomifun_api_types::PluginShareSourceDto::CurrentMount,
            candidate_id: None,
            expected_candidate_digest: None,
            mount_id: Some(mount_id),
            expected_mount_revision: Some(mount.summary.mount_revision),
            expected_target_digest: Some(current.artifact_digest),
            destination_path: args.output.display().to_string(),
            include_source: false,
        }
    } else {
        let ready = project.ready.as_ref().ok_or_else(|| {
            CliFailure::state("the Plugin Project has no Ready Candidate")
        })?;
        SharePluginRequest {
            project_id: project.summary.project_id.clone(),
            expected_project_revision: project.summary.project_revision,
            source: nomifun_api_types::PluginShareSourceDto::ReadyCandidate,
            candidate_id: Some(ready.candidate.candidate_id.clone()),
            expected_candidate_digest: Some(ready.candidate.candidate_digest.clone()),
            mount_id: None,
            expected_mount_revision: None,
            expected_target_digest: None,
            destination_path: args.output.display().to_string(),
            include_source: args.include_source,
        }
    };
    let response: ApiResponse<nomifun_api_types::DurableOperationDetailDto> = client
        .post_api(&format!("/api/plugin-projects/{project_id}/share"), &request)
        .await?;
    to_value(response)
}

async fn run_auto_apply(
    args: &PluginProjectArgs,
    enabled: bool,
    retry: bool,
) -> Result<Value, CliFailure> {
    let project_id = checked_segment(&args.project_id, "project_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let project = fetch_project(&client, &project_id).await?;
    if retry
        && project.summary.apply_mode
            != nomifun_api_types::PluginApplyModeDto::AutoCompatibleWhenIdle
    {
        return Err(CliFailure::state(
            "auto Apply retry requires an existing standing authorization",
        ));
    }
    let (linked_mount_id, expected_linked_mount_revision, expected_linked_target_digest) =
        if enabled {
            let mount_id = project.summary.linked_mount_id.as_deref().ok_or_else(|| {
                CliFailure::state("auto Apply requires an exact linked Mount")
            })?;
            let mount_id = checked_segment(mount_id, "mount_id")?;
            let mount = fetch_mount(&client, &mount_id).await?;
            let current = mount.summary.current.ok_or_else(|| {
                CliFailure::state("the linked Plugin Mount has no current target")
            })?;
            (
                Some(mount_id),
                Some(mount.summary.mount_revision),
                Some(current.artifact_digest),
            )
        } else {
            (None, None, None)
        };
    let request = SetPluginAutoApplyRequest {
        project_id: project.summary.project_id,
        expected_project_revision: project.summary.project_revision,
        expected_build_generation: project.summary.build_generation,
        linked_mount_id,
        expected_linked_mount_revision,
        expected_linked_target_digest,
        apply_mode: if enabled {
            nomifun_api_types::PluginApplyModeDto::AutoCompatibleWhenIdle
        } else {
            nomifun_api_types::PluginApplyModeDto::AskBeforeApply
        },
    };
    let response: ApiResponse<PluginProjectDetailDto> = client
        .put_api(&format!("/api/plugin-projects/{project_id}/auto-apply"), &request)
        .await?;
    to_value(response)
}

async fn run_apply(args: &PluginCandidateApplyArgs) -> Result<Value, CliFailure> {
    let project_id = checked_segment(&args.project_id, "project_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let project = fetch_project(&client, &project_id).await?;
    let ready = project.ready.clone().ok_or_else(|| {
        CliFailure::state("the Plugin Project has no Ready Candidate")
    })?;

    let target = match project.summary.linked_mount_id.as_deref() {
        Some(mount_id) => {
            let mount_id = checked_segment(mount_id, "mount_id")?;
            let mount = fetch_mount(&client, &mount_id).await?;
            let current = mount.summary.current.ok_or_else(|| {
                CliFailure::state(
                    "the linked Plugin Mount has no current target; \
                     restore or reinstall it before applying a replacement",
                )
            })?;
            ApplyPluginTargetDto::ExistingMount {
                mount_id,
                expected_mount_revision: mount.summary.mount_revision,
                expected_current_target_digest: current.artifact_digest,
            }
        }
        None => {
            let library: ApiResponse<PluginLibraryResponseDto> = client
                .get_api("/api/plugins")
                .await?;
            let library = require_data(library, "Plugin library")?;
            ApplyPluginTargetDto::InitialInstall {
                expected_library_revision: library.library_revision,
            }
        }
    };

    let request = ApplyPluginCandidateRequest {
        project_id: project.summary.project_id.clone(),
        expected_project_revision: project.summary.project_revision,
        expected_build_generation: project.summary.build_generation,
        candidate_id: ready.candidate.candidate_id,
        expected_candidate_digest: ready.candidate.candidate_digest,
        target,
        allow_breaking: args.allow_breaking,
        acknowledge_test_warning: args.acknowledge_test_warning,
    };
    let response: ApiResponse<PluginDetailDto> = client
        .post_api(
            &format!("/api/plugin-projects/{project_id}/apply"),
            &request,
        )
        .await?;
    to_value(response)
}

async fn run_restore(args: &PluginMountArgs) -> Result<Value, CliFailure> {
    let mount_id = checked_segment(&args.mount_id, "mount_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let mount = fetch_mount(&client, &mount_id).await?;
    let current = mount.summary.current.ok_or_else(|| {
        CliFailure::state("the Plugin Mount has no current target to replace")
    })?;
    let previous = mount.summary.previous.ok_or_else(|| {
        CliFailure::state("the Plugin Mount has no previous target to restore")
    })?;
    let request = RestorePluginPreviousRequest {
        mount_id: mount_id.clone(),
        expected_mount_revision: mount.summary.mount_revision,
        expected_current_target_digest: current.artifact_digest,
        expected_previous_target_digest: previous.artifact_digest,
    };
    let response: ApiResponse<PluginDetailDto> = client
        .post_api(
            &format!("/api/plugin-mounts/{mount_id}/restore"),
            &request,
        )
        .await?;
    to_value(response)
}

async fn run_set_enabled(
    args: &PluginMountArgs,
    enabled: bool,
) -> Result<Value, CliFailure> {
    let mount_id = checked_segment(&args.mount_id, "mount_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let mount = fetch_mount(&client, &mount_id).await?;
    let current = mount.summary.current.ok_or_else(|| {
        CliFailure::state("the Plugin Mount has no current target")
    })?;
    let request = SetPluginEnabledRequest {
        mount_id: mount_id.clone(),
        expected_mount_revision: mount.summary.mount_revision,
        expected_current_target_digest: current.artifact_digest,
        enabled,
    };
    let response: ApiResponse<PluginDetailDto> = client
        .put_api(
            &format!("/api/plugin-mounts/{mount_id}/enabled"),
            &request,
        )
        .await?;
    to_value(response)
}

async fn run_retry_mount(args: &PluginMountArgs) -> Result<Value, CliFailure> {
    let mount_id = checked_segment(&args.mount_id, "mount_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let mount = fetch_mount(&client, &mount_id).await?;
    let current = mount.summary.current.ok_or_else(|| {
        CliFailure::state("the Plugin Mount has no current target")
    })?;
    let request = RetryPluginRequest {
        mount_id: mount_id.clone(),
        expected_mount_revision: mount.summary.mount_revision,
        expected_current_target_digest: current.artifact_digest,
    };
    let response: ApiResponse<PluginDetailDto> = client
        .post_api(
            &format!("/api/plugin-mounts/{mount_id}/retry"),
            &request,
        )
        .await?;
    to_value(response)
}

async fn run_uninstall_mount(args: &PluginMountArgs) -> Result<Value, CliFailure> {
    let mount_id = checked_segment(&args.mount_id, "mount_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let mount = fetch_mount(&client, &mount_id).await?;
    let current = mount.summary.current.ok_or_else(|| {
        CliFailure::state("the Plugin Mount has no current target")
    })?;
    let request = UninstallPluginRequest {
        mount_id: mount_id.clone(),
        expected_mount_revision: mount.summary.mount_revision,
        expected_current_target_digest: current.artifact_digest,
    };
    let response: ApiResponse<PluginDetailDto> = client
        .post_api(
            &format!("/api/plugin-mounts/{mount_id}/uninstall"),
            &request,
        )
        .await?;
    to_value(response)
}

async fn run_delete_mount_data(args: &PluginMountArgs) -> Result<Value, CliFailure> {
    let mount_id = checked_segment(&args.mount_id, "mount_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let mount = fetch_mount(&client, &mount_id).await?;
    let request = DeletePluginDataRequest {
        mount_id: mount_id.clone(),
        expected_mount_revision: mount.summary.mount_revision,
        expected_lifecycle: nomifun_api_types::PluginLifecycleDto::UninstalledDataRetained,
        expected_data_revision: mount.summary.mount_revision,
    };
    if mount.summary.lifecycle
        != nomifun_api_types::PluginLifecycleDto::UninstalledDataRetained
    {
        return Err(CliFailure::state(
            "Plugin Mount runtime data can only be deleted after uninstall",
        ));
    }
    let response: ApiResponse<()> = client
        .delete_api(
            &format!("/api/plugin-mounts/{mount_id}/data"),
            &request,
        )
        .await?;
    to_value(response)
}

async fn run_miniapp_list(args: &MiniAppListArgs) -> Result<Value, CliFailure> {
    let client = HeadlessClient::new(&args.connection)?;
    let response: ApiResponse<MiniAppLibraryResponseDto> =
        client.get_api("/api/miniapps").await?;
    to_value(response)
}

async fn run_miniapp_show(args: &MiniAppShowArgs) -> Result<Value, CliFailure> {
    let miniapp_id = checked_segment(&args.miniapp_id, "miniapp_id")?;
    let client = HeadlessClient::new(&args.connection)?;
    let response: ApiResponse<MiniAppWorkshopDto> = client
        .get_api(&format!("/api/miniapps/{miniapp_id}/workshop"))
        .await?;
    to_value(response)
}

async fn fetch_project(
    client: &HeadlessClient,
    project_id: &str,
) -> Result<PluginProjectDetailDto, CliFailure> {
    let response: ApiResponse<PluginProjectDetailDto> = client
        .get_api(&format!("/api/plugin-projects/{project_id}"))
        .await?;
    require_data(response, "Plugin Project")
}

async fn fetch_mount(
    client: &HeadlessClient,
    mount_id: &str,
) -> Result<PluginDetailDto, CliFailure> {
    let response: ApiResponse<PluginDetailDto> = client
        .get_api(&format!("/api/plugin-mounts/{mount_id}"))
        .await?;
    require_data(response, "Plugin Mount")
}

async fn fetch_owner_id(client: &HeadlessClient) -> Result<String, CliFailure> {
    let response: AuthUserResponse = client
        .get_json("/api/auth/user")
        .await?;
    if !response.success {
        return Err(CliFailure::protocol(
            "the authentication endpoint returned success=false",
        ));
    }
    Ok(response.user.user_id)
}

fn require_data<T>(
    response: ApiResponse<T>,
    resource: &str,
) -> Result<T, CliFailure> {
    if !response.success {
        return Err(CliFailure::protocol(format!(
            "{resource} returned success=false"
        )));
    }
    response.data.ok_or_else(|| {
        CliFailure::protocol(format!("{resource} response has no data"))
    })
}

fn checked_segment(value: &str, field: &str) -> Result<String, CliFailure> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.chars().any(|character| {
            character.is_control()
                || matches!(character, '/' | '\\' | '?' | '#')
        })
    {
        return Err(CliFailure::usage(format!(
            "{field} must be a non-empty single path segment"
        )));
    }
    Ok(value.to_owned())
}

fn validate_digest(value: &str, field: &str) -> Result<(), CliFailure> {
    let valid = value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit());
    if !valid {
        return Err(CliFailure::usage(format!(
            "{field} must be a 64-character hexadecimal digest"
        )));
    }
    Ok(())
}

fn to_value<T: Serialize>(value: T) -> Result<Value, CliFailure> {
    serde_json::to_value(value)
        .map_err(|error| CliFailure::protocol(error.to_string()))
}

fn finish(result: Result<Value, CliFailure>) -> ExitCode {
    match result {
        Ok(value) => print_json(&value, ExitCode::SUCCESS),
        Err(error) => {
            let (response, code) = error.into_response();
            print_json(&response, ExitCode::from(code))
        }
    }
}

fn print_json<T: Serialize>(value: &T, exit_code: ExitCode) -> ExitCode {
    match serde_json::to_string_pretty(value) {
        Ok(json) => {
            println!("{json}");
            exit_code
        }
        Err(error) => {
            let fallback = ErrorResponse::new(
                format!("failed to serialize CLI JSON output: {error}"),
                "CLI_SERIALIZATION_ERROR",
            );
            println!(
                "{}",
                serde_json::to_string(&fallback).unwrap_or_else(|_| {
                    "{\"success\":false,\"error\":\"CLI serialization failure\",\"code\":\"CLI_SERIALIZATION_ERROR\"}".to_owned()
                })
            );
            ExitCode::from(1)
        }
    }
}

#[derive(Debug)]
enum CliFailure {
    Usage(String),
    State(String),
    Configuration(String),
    Network(String),
    Protocol(String),
    Api(ErrorResponse),
}

impl CliFailure {
    fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    fn state(message: impl Into<String>) -> Self {
        Self::State(message.into())
    }

    fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }

    fn network(message: impl Into<String>) -> Self {
        Self::Network(message.into())
    }

    fn protocol(message: impl Into<String>) -> Self {
        Self::Protocol(message.into())
    }

    fn into_response(self) -> (ErrorResponse, u8) {
        match self {
            Self::Usage(message) => (
                ErrorResponse::new(message, "CLI_INVALID_INPUT"),
                2,
            ),
            Self::Configuration(message) => (
                ErrorResponse::new(message, "CLI_INVALID_CONFIGURATION"),
                2,
            ),
            Self::State(message) => (
                ErrorResponse::new(message, "CLI_STATE_INVALID"),
                1,
            ),
            Self::Network(message) => (
                ErrorResponse::new(message, "CLI_NETWORK_ERROR"),
                1,
            ),
            Self::Protocol(message) => (
                ErrorResponse::new(message, "CLI_PROTOCOL_ERROR"),
                1,
            ),
            Self::Api(response) => (response, 1),
        }
    }
}

struct HeadlessClient {
    client: Client,
    base_url: String,
    token: String,
}

impl HeadlessClient {
    fn new(args: &HeadlessConnectionArgs) -> Result<Self, CliFailure> {
        let base_url = args
            .url
            .clone()
            .or_else(|| std::env::var("NOMIFUN_URL").ok())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_URL.to_owned());
        let base_url = base_url.trim_end_matches('/').to_owned();
        let parsed = reqwest::Url::parse(&base_url).map_err(|error| {
            CliFailure::configuration(format!(
                "invalid NomiFun base URL {base_url:?}: {error}"
            ))
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(CliFailure::configuration(
                "NomiFun base URL must use http or https",
            ));
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(CliFailure::configuration(
                "NomiFun base URL must not contain a query or fragment",
            ));
        }

        let token = args
            .token
            .clone()
            .or_else(|| std::env::var("NOMIFUN_ACCESS_TOKEN").ok())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                CliFailure::configuration(
                    "no access token: pass --token or set NOMIFUN_ACCESS_TOKEN",
                )
            })?;
        if token.trim() != token {
            return Err(CliFailure::configuration(
                "access token must not have surrounding whitespace",
            ));
        }

        let client = Client::builder()
            .user_agent(concat!("nomicore-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| {
                CliFailure::configuration(format!(
                    "cannot initialize HTTP client: {error}"
                ))
            })?;
        Ok(Self {
            client,
            base_url,
            token,
        })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    async fn get_api<T: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<ApiResponse<T>, CliFailure> {
        let endpoint = self.endpoint(path);
        self.decode_api(
            self.client
                .get(&endpoint)
                .bearer_auth(&self.token),
            endpoint,
        )
        .await
    }

    async fn post_api<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<ApiResponse<T>, CliFailure> {
        let endpoint = self.endpoint(path);
        self.decode_api(
            self.client
                .post(&endpoint)
                .bearer_auth(&self.token)
                .json(body),
            endpoint,
        )
        .await
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, CliFailure> {
        let endpoint = self.endpoint(path);
        self.decode_json(
            self.client
                .get(&endpoint)
                .bearer_auth(&self.token),
            endpoint,
        )
        .await
    }

    async fn put_api<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<ApiResponse<T>, CliFailure> {
        let endpoint = self.endpoint(path);
        self.decode_json(
            self.client
                .put(&endpoint)
                .bearer_auth(&self.token)
                .json(body),
            endpoint,
        )
        .await
    }

    async fn delete_api<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<ApiResponse<T>, CliFailure> {
        let endpoint = self.endpoint(path);
        self.decode_json(
            self.client
                .delete(&endpoint)
                .bearer_auth(&self.token)
                .json(body),
            endpoint,
        )
        .await
    }

    async fn decode_api<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
        endpoint: String,
    ) -> Result<ApiResponse<T>, CliFailure> {
        self.decode_json(request, endpoint).await
    }

    async fn decode_json<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
        endpoint: String,
    ) -> Result<T, CliFailure> {
        let response = request
            .send()
            .await
            .map_err(|error| CliFailure::network(format!("{endpoint}: {error}")))?;
        let status = response.status();
        let body = response.bytes().await.map_err(|error| {
            CliFailure::network(format!(
                "read response from {endpoint}: {error}"
            ))
        })?;
        if !status.is_success() {
            return Err(api_failure(status, &body));
        }
        serde_json::from_slice(&body).map_err(|error| {
            CliFailure::protocol(format!(
                "invalid JSON response from {endpoint}: {error}"
            ))
        })
    }
}

fn api_failure(status: StatusCode, body: &[u8]) -> CliFailure {
    if let Ok(response) = serde_json::from_slice::<ErrorResponse>(body) {
        return CliFailure::Api(response);
    }
    CliFailure::Api(ErrorResponse::new(
        format!("NomiFun API returned HTTP {status}"),
        "CLI_HTTP_ERROR",
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthUserResponse {
    success: bool,
    user: AuthUser,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthUser {
    user_id: String,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct SourcePathOutput {
    project_id: String,
    source_state: nomifun_api_types::PluginProjectSourceStateDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_snapshot_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dependency_lock_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    managed_relative_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::HeadlessConnectionArgs;
    use std::path::Path;

    fn connection(url: &str, token: &str) -> HeadlessConnectionArgs {
        HeadlessConnectionArgs {
            url: Some(url.to_owned()),
            token: Some(token.to_owned()),
        }
    }

    #[test]
    fn connection_rejects_invalid_url_and_missing_token() {
        let invalid = match HeadlessClient::new(&connection(
            "ftp://example.test",
            "secret",
        )) {
            Ok(_) => panic!("ftp must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(invalid, CliFailure::Configuration(_)));

        let missing = match HeadlessClient::new(&HeadlessConnectionArgs {
            url: Some("http://127.0.0.1:1".to_owned()),
            token: Some(" ".to_owned()),
        }) {
            Ok(_) => panic!("blank token must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(missing, CliFailure::Configuration(_)));
    }

    #[test]
    fn path_segments_and_digests_fail_closed() {
        assert!(checked_segment("project-1", "project_id").is_ok());
        assert!(checked_segment("../escape", "project_id").is_err());
        assert!(checked_segment("a/b", "project_id").is_err());
        assert!(validate_digest(&"a".repeat(64), "digest").is_ok());
        assert!(validate_digest("short", "digest").is_err());
    }

    #[test]
    fn cli_failures_use_usage_two_and_operation_one() {
        let (_, usage_code) =
            CliFailure::usage("bad input").into_response();
        assert_eq!(usage_code, 2);

        let (_, state_code) =
            CliFailure::state("missing candidate").into_response();
        assert_eq!(state_code, 1);

        let (_, api_code) = CliFailure::Api(ErrorResponse::new(
            "server rejected request",
            "CONFLICT",
        ))
        .into_response();
        assert_eq!(api_code, 1);
    }

    #[test]
    fn source_path_uses_the_managed_authoring_layout() {
        let root = std::env::temp_dir().join("nomifun-plugin-source-layout");
        let path = root
            .join(PLUGIN_PLATFORM_DIRECTORY)
            .join(PLUGIN_AUTHORING_DIRECTORY)
            .join("sources")
            .join("owner")
            .join("projects")
            .join("project")
            .join("source");
        assert_eq!(
            path.strip_prefix(&root).unwrap(),
            Path::new(PLUGIN_PLATFORM_DIRECTORY)
                .join(PLUGIN_AUTHORING_DIRECTORY)
                .join("sources")
                .join("owner")
                .join("projects")
                .join("project")
                .join("source")
        );
    }
}
