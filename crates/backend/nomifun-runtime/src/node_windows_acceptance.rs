//! Windows native official distribution + offline npm acceptance. The CLI
//! prefix, cache and npmrc are unique fixtures; the real login profile is kept.
use crate::{ToolId, managed::specification};
use std::{
    collections::HashMap,
    fs::{self, File},
    path::{Path, PathBuf},
};
const TEST: &str = "node_windows_acceptance::official_windows_node_npm_literal_cli_survives_restart_and_cache_repair";
const ROOT: &str = "NOMIFUN_WINDOWS_NODE_FIXTURE";
fn archive_fixture(root: &Path) -> PathBuf {
    let path = root.join("local package 中文.tgz");
    let compressor =
        flate2::write::GzEncoder::new(File::create(&path).unwrap(), flate2::Compression::fast());
    let mut archive = tar::Builder::new(compressor);
    for (name,bytes,mode) in [
        ("package/package.json",br#"{"name":"nomifun-windows-local-fixture","version":"1.0.0","bin":{"nomifun-windows-local-fixture":"cli.cjs"},"scripts":{"postinstall":"node postinstall.cjs"}}"#.as_slice(),0o644),
        ("package/cli.cjs",b"#!/usr/bin/env node\nconsole.log(JSON.stringify({argv:process.argv.slice(2),profile:process.env.USERPROFILE,cwd:process.cwd(),execPath:process.execPath}));\n".as_slice(),0o755),
        ("package/postinstall.cjs",b"require('fs').writeFileSync(process.env.NOMIFUN_FIXTURE_PROOF,JSON.stringify({profile:process.env.USERPROFILE,execPath:process.execPath}));\n".as_slice(),0o644),
    ] {
        let mut header=tar::Header::new_gnu();header.set_mode(mode);header.set_size(bytes.len() as u64);header.set_cksum();
        archive.append_data(&mut header,name,bytes).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap();
    path
}
fn system_path() -> String {
    let windows = PathBuf::from(std::env::var_os("SystemRoot").expect("Windows system root"));
    std::env::join_paths([
        windows.join("System32"),
        windows.join("System32/WindowsPowerShell/v1.0"),
    ])
    .unwrap()
    .into_string()
    .unwrap()
}
fn environment(root: &Path) -> HashMap<String, String> {
    HashMap::from([
        ("PATH".into(), system_path()),
        ("USERPROFILE".into(), std::env::var("USERPROFILE").unwrap()),
        (
            "NPM_CONFIG_PREFIX".into(),
            root.join("user-tools").to_str().unwrap().into(),
        ),
        (
            "NPM_CONFIG_USERCONFIG".into(),
            root.join("fixture.npmrc").to_str().unwrap().into(),
        ),
        (
            "NPM_CONFIG_CACHE".into(),
            root.join("npm-cache").to_str().unwrap().into(),
        ),
        (
            "NPM_CONFIG_REGISTRY".into(),
            "http://127.0.0.1:9/never-network".into(),
        ),
        ("NPM_CONFIG_OFFLINE".into(), "true".into()),
        (
            "NOMIFUN_FIXTURE_PROOF".into(),
            root.join("postinstall.json").to_str().unwrap().into(),
        ),
    ])
}
async fn execute(root: &Path, command: &str, args: &[String]) -> serde_json::Value {
    let launch = crate::prepare_process_launch(command, args, root, &environment(root))
        .await
        .unwrap();
    assert_eq!(
        launch
            .environment
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("USERPROFILE"))
            .unwrap()
            .1,
        &std::env::var("USERPROFILE").unwrap()
    );
    let mut child = nomi_process_runtime::ChildProcessBuilder::clean_cli(&launch.program);
    child
        .args(&launch.args)
        .env_clear()
        .envs(&launch.environment)
        .current_dir(root);
    let output = tokio::time::timeout(std::time::Duration::from_secs(90), child.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "{command}: {}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line).ok())
        .unwrap_or(serde_json::Value::Null)
}
fn seed(root: &Path, archive: &Path) {
    let identity = crate::embed::identity(ToolId::Node);
    let destination = root
        .join("data/runtime")
        .join(format!("node-{}-{}", identity.version, identity.sha256));
    fs::create_dir_all(&destination).unwrap();
    fs::copy(archive, destination.join("approved.asset")).unwrap();
}
#[tokio::test]
#[ignore = "explicit native official Windows Node extraction and local offline npm installation"]
async fn official_windows_node_npm_literal_cli_survives_restart_and_cache_repair() {
    if let Some(root) = std::env::var_os(ROOT) {
        let root = PathBuf::from(root);
        assert!(root.is_absolute());
        assert_eq!(
            fs::read_to_string(root.join("fixture-owned")).unwrap(),
            "windows-node-acceptance.v1"
        );
        crate::init(root.join("data"));
        let phase = std::env::var("NOMIFUN_WINDOWS_NODE_PHASE").unwrap();
        for directory in std::env::split_paths(&system_path()) {
            assert!(crate::resolve_command_in("node", &directory).is_none());
            assert!(crate::resolve_command_in("npm", &directory).is_none());
        }
        if phase == "install" {
            execute(
                &root,
                "npm",
                &[
                    "install".into(),
                    "--global".into(),
                    "--offline".into(),
                    "--foreground-scripts".into(),
                    "--no-audit".into(),
                    "--no-fund".into(),
                    root.join("local package 中文.tgz").to_str().unwrap().into(),
                ],
            )
            .await;
            let proof: serde_json::Value =
                serde_json::from_slice(&fs::read(root.join("postinstall.json")).unwrap()).unwrap();
            assert_eq!(proof["profile"], std::env::var("USERPROFILE").unwrap());
            assert!(
                proof["execPath"]
                    .as_str()
                    .unwrap()
                    .contains("distribution-v1")
            );
            assert!(
                root.join("user-tools/nomifun-windows-local-fixture.cmd")
                    .is_file()
            );
        } else {
            execute(&root, "node", &["--version".into()]).await;
        }
        let args: Vec<String> = [
            "",
            "中文 空格",
            "double\"quote",
            "single'quote",
            "% ! & | < > ^ $ $(throw SHOULD_NOT_EXECUTE)",
            "trailing\\",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let result = execute(&root, "nomifun-windows-local-fixture", &args).await;
        assert_eq!(result["argv"], serde_json::json!(args));
        assert_eq!(result["profile"], std::env::var("USERPROFILE").unwrap());
        let mut npx = vec![
            "--no-install".into(),
            "--offline".into(),
            "nomifun-windows-local-fixture".into(),
        ];
        npx.extend(args.clone());
        assert_eq!(
            execute(&root, "npx", &npx).await["argv"],
            serde_json::json!(args)
        );
        if phase == "install" {
            let node = crate::resolve_existing_tool(ToolId::Node).await.unwrap();
            let distribution = node.executable_path.parent().unwrap();
            assert!(distribution.starts_with(root.join("data/runtime")));
            assert!(distribution.ends_with("distribution-v1"));
            fs::remove_dir_all(distribution).unwrap();
            assert!(crate::resolve_existing_tool(ToolId::Node).await.is_err());
            assert_eq!(
                execute(&root, "nomifun-windows-local-fixture", &args).await["argv"],
                serde_json::json!(args)
            );
        }
        if phase == "restart" {
            execute(
                &root,
                "npm",
                &[
                    "uninstall".into(),
                    "--global".into(),
                    "--offline".into(),
                    "--no-audit".into(),
                    "--no-fund".into(),
                    "nomifun-windows-local-fixture".into(),
                ],
            )
            .await;
            assert!(
                !root
                    .join("user-tools/nomifun-windows-local-fixture.cmd")
                    .exists()
            );
        }
        return;
    }
    let spec = specification(ToolId::Node);
    let asset = spec
        .asset(env!("NOMIFUN_RUNTIME_TARGET"), "default")
        .unwrap();
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap().join(".cargo"));
    let archive = cargo_home
        .join("nomifun-component-cache/node")
        .join(&asset.sha256)
        .join("approved.asset");
    assert!(
        archive.is_file(),
        "build.rs verified official archive required"
    );
    let temporary = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temporary.path()).unwrap();
    fs::write(root.join("fixture-owned"), "windows-node-acceptance.v1").unwrap();
    fs::write(root.join("fixture.npmrc"), "").unwrap();
    archive_fixture(&root);
    seed(&root, &archive);
    for phase in ["install", "retained", "restart"] {
        if phase == "retained" {
            let cache = root.join("data/runtime");
            assert!(cache.starts_with(&root));
            fs::remove_dir_all(cache).unwrap();
            seed(&root, &archive);
        }
        let mut child =
            nomi_process_runtime::ChildProcessBuilder::clean_cli(std::env::current_exe().unwrap());
        child
            .args(["--ignored", "--exact", TEST, "--nocapture"])
            .env(ROOT, &root)
            .env("NOMIFUN_WINDOWS_NODE_PHASE", phase)
            .env("PATH", system_path())
            .env_remove("NOMIFUN_NODE_PATH")
            .env_remove("NOMIFUN_BUN_PATH")
            .env_remove("NODE_OPTIONS");
        let output = child.output().await.unwrap();
        assert!(
            output.status.success(),
            "phase {phase}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
