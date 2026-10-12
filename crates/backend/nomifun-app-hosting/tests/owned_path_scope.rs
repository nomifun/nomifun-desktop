//! Actual Windows junctions, confined to an isolated temporary fixture.
#![cfg(windows)]
use std::{fs, path::{Path, PathBuf}};
use nomifun_app_hosting::{AppMetadata, AppOperation, HostingOwner, ImportKind, LocalConnection};

struct Junction(PathBuf);
impl Drop for Junction {
    fn drop(&mut self) { let _ = junction::delete(&self.0); }
}

async fn fixture(root: &Path) -> (std::sync::Arc<HostingOwner>, nomifun_app_hosting::AppRecord) {
    let source = root.join("original");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("server.cjs"), "require('fs').writeFileSync('started','must stay in managed copy');setInterval(()=>{},1000)").unwrap();
    let host = HostingOwner::new(root.join("hosting"), LocalConnection {
        host_url: "http://127.0.0.1:5177".into(), local_trust: "scope-test-secret".into(),
    }).await.unwrap();
    let app = host.import(&source, None, ImportKind::Artifact).await.unwrap();
    (host, app)
}

#[tokio::test]
async fn replaced_application_root_cannot_write_preset_overwrite_or_start_in_external_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let (host, app) = fixture(temporary.path()).await;
    let external = temporary.path().join("external-original");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("index.html"), "EXTERNAL_ORIGINAL").unwrap();
    fs::write(external.join("server.cjs"), "require('fs').writeFileSync('started','external execution');setInterval(()=>{},1000)").unwrap();
    assert!(dunce::canonicalize(&app.directory).unwrap().starts_with(dunce::canonicalize(temporary.path()).unwrap()));
    fs::rename(&app.directory, temporary.path().join("retained-managed-copy")).unwrap();
    junction::create(&external, &app.directory).unwrap();
    let _junction = Junction(app.directory.clone());
    let mut settings = app.settings.clone();
    settings.start_command = Some("node .nomifun-static-server.cjs".into());
    settings.output_directory = Some(".".into());
    assert!(host.update(&app.id, app.name.clone(), settings).await.is_err(), "a linked app root must not become the new preset anchor");
    assert!(!external.join(".nomifun-static-server.cjs").exists());
    let replacement = temporary.path().join("replacement");
    fs::create_dir(&replacement).unwrap();
    fs::write(replacement.join("index.html"), "OVERWRITE_MUST_STAY_OWNED").unwrap();
    assert!(host.overwrite(&app.id, &replacement).await.is_err());
    assert_eq!(fs::read_to_string(external.join("index.html")).unwrap(), "EXTERNAL_ORIGINAL");
    assert!(host.operate(&app.id, AppOperation::Start).await.is_err());
    assert!(!external.join("started").exists());
    assert!(host.delete(&app.id).await.is_err());
    assert_eq!(fs::read_to_string(external.join("index.html")).unwrap(), "EXTERNAL_ORIGINAL");
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn replaced_apps_ancestor_cannot_turn_external_child_into_owned_deletion_target() {
    let temporary = tempfile::tempdir().unwrap();
    let (host, app) = fixture(temporary.path()).await;
    let apps = temporary.path().join("hosting/apps");
    let external = temporary.path().join("external-apps");
    let external_child = external.join(&app.id);
    fs::create_dir_all(&external_child).unwrap();
    fs::write(external_child.join("user-data.txt"), "EXTERNAL_USER_DATA").unwrap();
    assert!(dunce::canonicalize(&apps).unwrap().starts_with(dunce::canonicalize(temporary.path()).unwrap()));
    fs::rename(&apps, temporary.path().join("retained-apps")).unwrap();
    junction::create(&external, &apps).unwrap();
    let _junction = Junction(apps);
    assert!(host.delete(&app.id).await.is_err(), "canonicalizing an apps junction must not authorize deletion of its external child");
    assert_eq!(fs::read_to_string(external_child.join("user-data.txt")).unwrap(), "EXTERNAL_USER_DATA");
    assert!(host.get(&app.id).await.is_ok(), "failed deletion must retain the same registration");
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn real_build_cannot_change_static_output_into_an_external_junction_before_start() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("original");
    let external = temporary.path().join("external-output");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&external).unwrap();
    fs::write(external.join("index.html"), "EXTERNAL_OUTPUT_MUST_NOT_BE_HOSTED").unwrap();
    fs::write(source.join("package.json"), r#"{"scripts":{"build":"node build.cjs"}}"#).unwrap();
    fs::write(source.join("build.cjs"), "const fs=require('fs');fs.symlinkSync(process.env.EXTERNAL_OUTPUT,'dist','junction');fs.writeFileSync('build-finished','yes');").unwrap();
    let host = HostingOwner::new(temporary.path().join("hosting"), LocalConnection {
        host_url: "http://127.0.0.1:5177".into(), local_trust: "scope-test-secret".into(),
    }).await.unwrap();
    let mut app = host.import(&source, None, ImportKind::Source).await.unwrap();
    app.settings.install_command = None;
    app.settings.build_command = Some("node build.cjs".into());
    app.settings.start_command = Some("node .nomifun-static-server.cjs".into());
    app.settings.output_directory = Some("dist".into());
    app.settings.environment.insert("EXTERNAL_OUTPUT".into(), external.to_string_lossy().into_owned());
    host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
    let result = host.operate(&app.id, AppOperation::Use).await;
    let _junction = Junction(app.directory.join("dist"));
    assert!(app.directory.join("build-finished").exists(), "the actual Node build must have completed");
    assert!(junction::exists(app.directory.join("dist")).unwrap(), "the actual Node build must have created the junction");
    assert!(result.is_err(), "start must revalidate output ownership after the build changed its path");
    let failed = host.get(&app.id).await.unwrap();
    assert_eq!(failed.state, nomifun_app_hosting::AppState::Failed);
    assert!(failed.pid.is_none(), "no static process may start against external output");
    assert_eq!(fs::read_to_string(external.join("index.html")).unwrap(), "EXTERNAL_OUTPUT_MUST_NOT_BE_HOSTED");
    assert!(source.join("build.cjs").exists());
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn generated_project_discovery_refuses_real_junction_without_touching_external_files() {
    let temporary = tempfile::tempdir().unwrap();
    let host = HostingOwner::new(temporary.path().join("hosting"), LocalConnection {
        host_url: "http://127.0.0.1:5177".into(), local_trust: "scope-test-secret".into(),
    }).await.unwrap();
    let app = host.create("新应用".into(), AppMetadata::default()).await.unwrap();
    let external = temporary.path().join("external-source");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("index.html"), "EXTERNAL_USER_SOURCE").unwrap();
    junction::create(&external, app.directory.join("snake-game")).unwrap();
    let _junction = Junction(app.directory.join("snake-game"));
    assert!(host.update_patch_details(&app.id, None, nomifun_app_hosting::AppDetails::default(),
        nomifun_app_hosting::AppSettingsPatch::default(), true).await.is_err());
    assert_eq!(host.get(&app.id).await.unwrap().state, nomifun_app_hosting::AppState::Stopped,
        "configuration detection failure does not invent a process lifecycle failure");
    assert!(host.operate(&app.id, AppOperation::Preview).await.is_err());
    assert!(host.get(&app.id).await.unwrap().settings.web_url.is_none());
    assert!(!external.join(".nomifun-static-server.cjs").exists());
    assert_eq!(fs::read_to_string(external.join("index.html")).unwrap(), "EXTERNAL_USER_SOURCE");
    host.shutdown().await.unwrap();
}
