use super::*;
use nomifun_api_types::{
    ImportPluginRuntimeArtifactRequest, ImportPluginRuntimeBackupRequest, ImportPluginRuntimeShareRequest,
    PluginRuntimeShareContentDto, SharePluginRuntimeRequest,
};
use nomifun_plugin_platform::runtime::{PluginRuntimeShareBundleFilesystem, PluginRuntimeWholeAppBackupFilesystem};
use std::{
    fs,
    io::Read,
    path::{Component, Path as FsPath},
};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ImportSource {
    pub kind: String,
    pub path: String,
    pub digest: String,
    pub release_digest: String,
    pub editable: bool,
    pub includes_data: bool,
    pub requires_service: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InspectRequest {
    source_path: Option<String>,
    filename: Option<String>,
    content: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExportRequest {
    destination_path: String,
    #[serde(default)]
    backup: bool,
}

#[derive(Serialize)]
pub(super) struct ExportResult {
    pub exported: bool,
    pub resumed: bool,
}

pub(super) async fn inspect(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<InspectRequest>,
) -> Result<Json<ApiResponse<Draft>>, AppError> {
    let service = service(&state)?;
    let mut draft = Draft {
        id: uuid::Uuid::now_v7().to_string(),
        revision: 0,
        name: request
            .filename
            .as_deref()
            .and_then(|v| FsPath::new(v).file_stem())
            .and_then(|v| v.to_str())
            .unwrap_or("Plugin")
            .into(),
        description: String::new(),
        html: String::new(),
        service_source: None,
        source_manifest: None,
        messages: vec![],
        status: "ready".into(),
        error: None,
        miniapp_id: None,
        base_release_digest: None,
        base_source_digest: None,
        updated_at: nomifun_common::now_ms(),
        import: None,
    };
    if let Some(content) = request.content {
        authoring::validate_html(&content)?;
        draft.html = content;
    } else {
        let source = PathBuf::from(
            request
                .source_path
                .ok_or_else(|| invalid("Choose a Plugin file"))?,
        );
        if !source.is_absolute() {
            return Err(invalid("Choose an absolute local file path"));
        }
        let metadata = fs::symlink_metadata(&source)
            .map_err(|_| invalid("The selected file could not be read"))?;
        if metadata.file_type().is_symlink() {
            return Err(invalid("Choose the original Plugin file"));
        }
        if metadata.is_file()
            && matches!(
                source
                    .extension()
                    .and_then(|v| v.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("html" | "htm")
            )
        {
            if metadata.len() > 2_000_000 {
                return Err(invalid("This HTML file is too large"));
            }
            draft.html =
                fs::read_to_string(&source).map_err(|_| invalid("The HTML file must use UTF-8"))?;
            authoring::validate_html(&draft.html)?;
            draft.name = source
                .file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or("Plugin")
                .to_owned();
        } else {
            let source = if metadata.is_file() {
                let stage = service.root.join("miniapp-imports").join(&draft.id);
                let path = source.clone();
                let target = stage.clone();
                let unpacked = tokio::task::spawn_blocking(move || extract_archive(&path, &target))
                    .await
                    .map_err(internal)
                    .and_then(|result| result);
                if let Err(error) = unpacked {
                    service.cleanup_import_stage(&draft)?;
                    return Err(error);
                }
                stage
            } else if metadata.is_dir() {
                source
            } else {
                return Err(invalid("This file type is not supported"));
            };
            if let Err(error) = inspect_directory(&source, &mut draft) {
                service.cleanup_import_stage(&draft)?;
                return Err(error);
            }
        }
    }
    let library = service
        .application
        .library(user.id.as_str())
        .await
        .map_err(application_error)?;
    let original = draft.name.clone();
    let mut number = 2;
    while library
        .miniapps
        .iter()
        .any(|a| a.display_name == draft.name)
    {
        draft.name = format!("{original} ({number})");
        number += 1;
    }
    if draft.name.chars().count() > 120 {
        draft.name = draft.name.chars().take(120).collect();
    }
    if let Err(error) = service.put_draft(user.id.as_str(), &mut draft).await {
        service.cleanup_import_stage(&draft)?;
        return Err(error);
    }
    Ok(Json(ApiResponse::ok(draft)))
}

fn inspect_directory(path: &FsPath, draft: &mut Draft) -> Result<(), AppError> {
    let filesystem = PluginRuntimeShareBundleFilesystem::default();
    if path.join("bundle.json").is_file() {
        let imported = filesystem
            .import_share_bundle(path)
            .map_err(|_| invalid("This Plugin package is damaged or incomplete"))?;
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(path.join("bundle.json")).map_err(internal)?)
                .map_err(internal)?;
        let artifact = &json["release"];
        draft.name = artifact["manifest"]["payload"]["display"]["name"]
            .as_str()
            .unwrap_or("Plugin")
            .into();
        draft.description = artifact["manifest"]["payload"]["display"]["description"]
            .as_str()
            .unwrap_or("")
            .into();
        draft.html = imported
            .release
            .files
            .iter()
            .find(|f| f.normalized_relative_path == "ui/index.html")
            .and_then(|f| String::from_utf8(f.bytes.clone()).ok())
            .unwrap_or_default();
        draft.import = Some(ImportSource {
            kind: "share".into(),
            path: path.to_string_lossy().into(),
            digest: json["bundle_digest"]
                .as_str()
                .ok_or_else(|| invalid("Invalid Plugin package"))?
                .into(),
            release_digest: artifact["artifact_digest"]
                .as_str()
                .ok_or_else(|| invalid("Invalid Plugin release"))?
                .into(),
            editable: imported.source.is_some(),
            includes_data: false,
            requires_service: imported
                .release
                .files
                .iter()
                .any(|f| f.normalized_relative_path == "service/main.mjs"),
        });
    } else if path.join("metadata.json").is_file() {
        // Full backup validation is still performed by the canonical importer.
        let validated = PluginRuntimeWholeAppBackupFilesystem::default()
            .import(path)
            .map_err(|_| invalid("This backup is damaged or incomplete"))?;
        let metadata = fs::read(path.join("metadata.json")).map_err(internal)?;
        let product: serde_json::Value =
            serde_json::from_slice(&fs::read(path.join("product.json")).map_err(internal)?)
                .map_err(internal)?;
        draft.name = product["display_name"].as_str().unwrap_or("Plugin").into();
        draft.html = validated
            .releases
            .get("active")
            .and_then(|release| {
                release
                    .files
                    .iter()
                    .find(|file| file.relative_path == "ui/index.html")
            })
            .and_then(|file| String::from_utf8(file.bytes.clone()).ok())
            .unwrap_or_default();
        draft.import = Some(ImportSource {
            kind: "backup".into(),
            path: path.to_string_lossy().into(),
            digest: nomifun_agent_contracts::digest_bytes(&metadata)
                .as_ref()
                .into(),
            release_digest: String::new(),
            editable: validated.source.is_some(),
            includes_data: true,
            requires_service: product["kind"].as_str() == Some("service"),
        });
    } else {
        let root = if path.join("artifact.json").is_file() {
            path.to_path_buf()
        } else {
            path.join("release")
        };
        let imported = filesystem
            .import_prebuilt_release(&root)
            .map_err(|_| invalid("Choose a Plugin sharing package, backup, or HTML file"))?;
        let artifact: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("artifact.json")).map_err(internal)?)
                .map_err(internal)?;
        draft.name = artifact["manifest"]["payload"]["display"]["name"]
            .as_str()
            .unwrap_or("Plugin")
            .into();
        draft.html = imported
            .files
            .iter()
            .find(|f| f.normalized_relative_path == "ui/index.html")
            .and_then(|f| String::from_utf8(f.bytes.clone()).ok())
            .unwrap_or_default();
        draft.import = Some(ImportSource {
            kind: "artifact".into(),
            path: root.to_string_lossy().into(),
            digest: artifact["artifact_digest"]
                .as_str()
                .ok_or_else(|| invalid("Invalid Plugin release"))?
                .into(),
            release_digest: String::new(),
            editable: false,
            includes_data: false,
            requires_service: imported
                .files
                .iter()
                .any(|f| f.normalized_relative_path == "service/main.mjs"),
        });
    }
    Ok(())
}

impl PluginRuntimeProductService {
    pub(super) fn cleanup_import_stage(&self, draft: &Draft) -> Result<(), AppError> {
        valid_id(&draft.id)?;
        let root = self.root.join("miniapp-imports");
        let stage = root.join(&draft.id);
        if !stage.exists() {
            return Ok(());
        }
        let base = fs::canonicalize(root).map_err(internal)?;
        let target = fs::canonicalize(&stage).map_err(internal)?;
        if !target.starts_with(&base) || target == base {
            return Err(invalid("Invalid Plugin import staging path"));
        }
        fs::remove_dir_all(stage).map_err(internal)
    }

    pub(super) async fn commit_import(
        &self,
        owner: &str,
        draft: &Draft,
    ) -> Result<PluginRuntimeWorkshopDto, AppError> {
        let source = draft
            .import
            .as_ref()
            .ok_or_else(|| invalid("Import source is missing"))?;
        let library = self
            .application
            .library(owner)
            .await
            .map_err(application_error)?;
        match source.kind.as_str() {
            "share" => self
                .application
                .import_share(
                    owner,
                    ImportPluginRuntimeShareRequest {
                        expected_library_revision: library.library_revision,
                        source_path: source.path.clone(),
                        expected_bundle_digest: source.digest.clone(),
                        expected_release_digest: source.release_digest.clone(),
                        display_name: draft.name.clone(),
                    },
                )
                .await
                .map_err(application_error),
            "artifact" => self
                .application
                .import_prebuilt(
                    owner,
                    ImportPluginRuntimeArtifactRequest {
                        expected_library_revision: library.library_revision,
                        source_path: source.path.clone(),
                        expected_artifact_digest: source.digest.clone(),
                        display_name: draft.name.clone(),
                    },
                )
                .await
                .map_err(application_error),
            "backup" => self
                .application
                .import_backup(
                    owner,
                    ImportPluginRuntimeBackupRequest {
                        expected_library_revision: library.library_revision,
                        source_path: source.path.clone(),
                        expected_backup_metadata_digest: source.digest.clone(),
                        display_name: draft.name.clone(),
                    },
                )
                .await
                .map_err(application_error),
            _ => Err(invalid("Unsupported Plugin import")),
        }
    }
}

pub(super) async fn export_file(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(request): Json<ExportRequest>,
) -> Result<Json<ApiResponse<ExportResult>>, AppError> {
    let service = service(&state)?;
    let _lock = service.mutations.lock().await;
    let destination = PathBuf::from(&request.destination_path);
    if !destination.is_absolute() || destination.exists() {
        return Err(invalid("Choose a new sharing file name"));
    }
    let mut workshop = service
        .application
        .workshop(user.id.as_str(), &id)
        .await
        .map_err(application_error)?;
    let resume = request.backup
        && matches!(
            workshop.miniapp.lifecycle,
            nomifun_api_types::PluginRuntimeLifecycleDto::Enabled
        );
    let export_root = service.root.join("miniapp-exports");
    fs::create_dir_all(&export_root).map_err(internal)?;
    let stage = export_root.join(uuid::Uuid::now_v7().to_string());
    if resume {
        workshop = service
            .application
            .set_enabled(
                user.id.as_str(),
                SetPluginRuntimeEnabledRequest {
                    miniapp_id: id.clone(),
                    expected_product_revision: workshop.miniapp.product_revision,
                    expected_pointer_revision: workshop.miniapp.releases.pointer_revision,
                    expected_active_release_digest: workshop
                        .miniapp
                        .releases
                        .active
                        .as_ref()
                        .map(|r| r.release_digest.clone()),
                    enabled: false,
                },
            )
            .await
            .map_err(application_error)?;
    }
    let result = async {
        if request.backup {
            service
                .application
                .export_backup(
                    user.id.as_str(),
                    nomifun_api_types::ExportPluginRuntimeBackupRequest {
                        miniapp_id: id.clone(),
                        expected_product_revision: workshop.miniapp.product_revision,
                        expected_lifecycle: nomifun_api_types::PluginRuntimeLifecycleDto::Disabled,
                        expected_pointer_revision: workshop.miniapp.releases.pointer_revision,
                        expected_config_revision: workshop.config.config_revision,
                        expected_credential_bindings_revision: workshop
                            .credential_bindings_revision,
                        destination_path: stage.to_string_lossy().into(),
                    },
                )
                .await
                .map_err(application_error)?;
        } else {
            let release = workshop
                .miniapp
                .releases
                .active
                .as_ref()
                .ok_or_else(|| invalid("Save the Plugin before sharing"))?;
            service
                .application
                .export_share(
                    user.id.as_str(),
                    SharePluginRuntimeRequest {
                        miniapp_id: id.clone(),
                        expected_product_revision: workshop.miniapp.product_revision,
                        expected_pointer_revision: workshop.miniapp.releases.pointer_revision,
                        content: PluginRuntimeShareContentDto::ActiveRelease,
                        release_id: release.release_id.clone(),
                        expected_release_digest: release.release_digest.clone(),
                        destination_path: stage.to_string_lossy().into(),
                        include_source: matches!(
                            workshop.source_state,
                            nomifun_api_types::PluginRuntimeProjectSourceStateDto::Editable
                        ),
                    },
                )
                .await
                .map_err(application_error)?;
        }
        let source = stage.clone();
        tokio::task::spawn_blocking(move || write_archive(&source, &destination))
            .await
            .map_err(internal)?
    }
    .await;
    let resumed = if resume {
        service
            .application
            .set_enabled(
                user.id.as_str(),
                SetPluginRuntimeEnabledRequest {
                    miniapp_id: id,
                    expected_product_revision: workshop.miniapp.product_revision,
                    expected_pointer_revision: workshop.miniapp.releases.pointer_revision,
                    expected_active_release_digest: workshop
                        .miniapp
                        .releases
                        .active
                        .as_ref()
                        .map(|r| r.release_digest.clone()),
                    enabled: true,
                },
            )
            .await
            .is_ok()
    } else {
        true
    };
    // This stage is created by this request under the application's export root.
    if stage.exists() {
        if let Err(error) = fs::remove_dir_all(&stage) {
            tracing::warn!(%error,"Plugin export staging cleanup failed");
        }
    }
    result?;
    Ok(Json(ApiResponse::ok(ExportResult {
        exported: true,
        resumed,
    })))
}

fn extract_archive(source: &FsPath, destination: &FsPath) -> Result<(), AppError> {
    let file = fs::File::open(source).map_err(internal)?;
    let file_size = file.metadata().map_err(internal)?.len();
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| invalid("This file is not a Plugin sharing package"))?;
    let backup = archive.file_names().any(|name| name == "metadata.json");
    let limit = if backup {
        4 * 1024 * 1024 * 1024u64
    } else {
        64 * 1024 * 1024
    };
    if file_size > limit {
        return Err(invalid("This Plugin package exceeds the supported size"));
    }
    if archive.len() > if backup { 32768 } else { 4096 } {
        return Err(invalid("This Plugin package has too many files"));
    }
    fs::create_dir_all(destination).map_err(internal)?;
    let mut total = 0u64;
    let mut names = std::collections::BTreeSet::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(internal)?;
        let raw = entry.name().to_owned();
        if raw.contains(['\\', ':']) || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err(invalid("Unsupported path in Plugin package"));
        }
        let path = entry
            .enclosed_name()
            .ok_or_else(|| invalid("Invalid path in Plugin package"))?
            .to_path_buf();
        if path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
            || !names.insert(raw.to_lowercase())
        {
            return Err(invalid("Ambiguous path in Plugin package"));
        }
        for part in path.components() {
            let text = part.as_os_str().to_string_lossy();
            if text.ends_with(['.', ' ']) {
                return Err(invalid("Unsupported file name in Plugin package"));
            }
        }
        let target = destination.join(path);
        if entry.is_dir() {
            fs::create_dir_all(target).map_err(internal)?;
            continue;
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| invalid("Package too large"))?;
        if total > limit {
            return Err(invalid("The unpacked Plugin exceeds the supported size"));
        }
        fs::create_dir_all(
            target
                .parent()
                .ok_or_else(|| invalid("Invalid package path"))?,
        )
        .map_err(internal)?;
        let expected = entry.size();
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(internal)?;
        let copied =
            std::io::copy(&mut (&mut entry).take(expected + 1), &mut output).map_err(internal)?;
        if copied != expected {
            return Err(invalid("The Plugin package is incomplete"));
        }
    }
    Ok(())
}

fn write_archive(source: &FsPath, destination: &FsPath) -> Result<(), AppError> {
    let parent = destination
        .parent()
        .ok_or_else(|| invalid("Choose a destination folder"))?;
    let staged = parent.join(format!(".nomifun-miniapp-{}.tmp", uuid::Uuid::now_v7()));
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(staged.clone());
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .map_err(internal)?;
    let mut zip = zip::ZipWriter::new(file);
    fn add(
        zip: &mut zip::ZipWriter<fs::File>,
        root: &FsPath,
        path: &FsPath,
    ) -> Result<(), AppError> {
        for entry in fs::read_dir(path).map_err(internal)? {
            let entry = entry.map_err(internal)?;
            let kind = entry.file_type().map_err(internal)?;
            if kind.is_symlink() {
                return Err(invalid("Cannot share linked files"));
            }
            if kind.is_dir() {
                let directory = entry.path();
                let name = directory
                    .strip_prefix(root)
                    .map_err(internal)?
                    .to_string_lossy()
                    .replace('\\', "/");
                zip.add_directory(name, zip::write::SimpleFileOptions::default())
                    .map_err(internal)?;
                add(zip, root, &directory)?;
            } else if kind.is_file() {
                let file_path = entry.path();
                let name = file_path
                    .strip_prefix(root)
                    .map_err(internal)?
                    .to_string_lossy()
                    .replace('\\', "/");
                zip.start_file(
                    name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .map_err(internal)?;
                let mut file = fs::File::open(file_path).map_err(internal)?;
                std::io::copy(&mut file, zip).map_err(internal)?;
            }
        }
        Ok(())
    }
    add(&mut zip, source, source)?;
    zip.finish()
        .map_err(internal)?
        .sync_all()
        .map_err(internal)?;
    // Publish only a complete archive, and never overwrite an existing file.
    fs::hard_link(&staged, destination).map_err(internal)?;
    Ok(())
}
