//! Opt-in real Node/npm regression with an offline package and an isolated user.
#![cfg(all(test, target_os = "macos"))]
use crate::{ToolId, managed::specification};
use std::{collections::HashMap, fs::{self, File}, path::{Path, PathBuf}};

const CHILD_ROOT: &str = "NOMIFUN_NODE_ACCEPTANCE_ROOT";
const CHILD_PHASE: &str = "NOMIFUN_NODE_ACCEPTANCE_PHASE";
const TEST: &str = "node_acceptance::official_node_offline_npm_lifecycle_argv_and_restart";

fn archive_fixture(root: &Path) -> PathBuf {
    let archive = root.join("fixture with 中文 spaces.tgz");
    let compressor = flate2::write::GzEncoder::new(File::create(&archive).unwrap(), flate2::Compression::fast());
    let mut tar = tar::Builder::new(compressor);
    let manifest = serde_json::json!({"name":"nomifun-local-fixture","version":"1.0.0","bin":{"nomifun-local-fixture":"cli.cjs"},"scripts":{"postinstall":"node postinstall.cjs"}}).to_string();
    for (path, data, mode) in [
        ("package/package.json", manifest.as_bytes(), 0o644),
        ("package/cli.cjs", b"#!/usr/bin/env node\nconsole.log(JSON.stringify({argv:process.argv.slice(2),home:process.env.HOME,cwd:process.cwd(),execPath:process.execPath}));\n".as_slice(), 0o755),
        ("package/postinstall.cjs", b"require('node:fs').writeFileSync(process.env.NOMIFUN_FIXTURE_PROOF, JSON.stringify({home:process.env.HOME,execPath:process.execPath}));\n".as_slice(), 0o644),
    ] {
        let mut header = tar::Header::new_gnu(); header.set_mode(mode); header.set_size(data.len() as u64); header.set_cksum();
        tar.append_data(&mut header, path, data).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap(); archive
}

fn seed_verified_archive(root: &Path, archive: &Path) {
    let identity = crate::embed::identity(ToolId::Node);
    let directory = root.join("data/runtime").join(format!("node-{}-{}", identity.version, identity.sha256));
    fs::create_dir_all(&directory).unwrap(); fs::copy(archive, directory.join("approved.asset")).unwrap();
}

async fn execute(root: &Path, command: &str, args: &[String]) -> serde_json::Value {
    let configured = HashMap::from([
        ("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()),
        ("HOME".into(), root.join("home").to_string_lossy().into_owned()),
        ("NPM_CONFIG_USERCONFIG".into(), root.join("home/.npmrc").to_string_lossy().into_owned()),
        ("NPM_CONFIG_CACHE".into(), root.join("npm-cache").to_string_lossy().into_owned()),
        ("NPM_CONFIG_REGISTRY".into(), "http://127.0.0.1:9/never-network".into()),
        ("NPM_CONFIG_OFFLINE".into(), "true".into()),
        ("NOMIFUN_FIXTURE_PROOF".into(), root.join("postinstall-proof.json").to_string_lossy().into_owned()),
    ]);
    let launch = crate::prepare_process_launch(command, args, root, &configured).await.unwrap();
    let mut process = nomi_process_runtime::ChildProcessBuilder::clean_cli(&launch.program);
    process.args(&launch.args).env_clear().envs(&launch.environment).current_dir(root);
    let output = tokio::time::timeout(std::time::Duration::from_secs(60), process.output()).await.unwrap().unwrap();
    assert!(output.status.success(), "{command}: status={} stdout={} stderr={}", output.status, String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    stdout.lines().rev().find_map(|line| serde_json::from_str(line).ok()).unwrap_or_else(|| serde_json::json!({"stdout":stdout}))
}

/// This regression uses the exact official archive already checksum-verified by
/// build.rs. No system Node/npm, registry, account, or npm pack is involved.
#[tokio::test]
#[ignore = "opt-in real official Node extraction and offline npm lifecycle; isolated subprocess user"]
async fn official_node_offline_npm_lifecycle_argv_and_restart() {
    if let Ok(root) = std::env::var(CHILD_ROOT) {
        let root = PathBuf::from(root); crate::init(root.join("data"));
        assert!(crate::resolve_command_in("node", Path::new("/usr/bin")).is_none());
        assert!(crate::resolve_command_in("npm", Path::new("/usr/bin")).is_none());
        let phase = std::env::var(CHILD_PHASE).unwrap();
        if phase == "install" {
            assert!(crate::managed::published_tool(ToolId::Node).is_none()); // only the verified archive exists, not a published tree
            let args = vec!["install".into(), "--global".into(), "--offline".into(), "--foreground-scripts".into(), "--no-audit".into(), "--no-fund".into(), "--update-notifier=false".into(), root.join("fixture with 中文 spaces.tgz").to_string_lossy().into_owned()];
            execute(&root, "npm", &args).await;
            let proof: serde_json::Value = serde_json::from_slice(&fs::read(root.join("postinstall-proof.json")).unwrap()).unwrap();
            assert_eq!(proof["home"], root.join("home").to_string_lossy().as_ref());
            assert!(proof["execPath"].as_str().unwrap().contains("distribution-v1/bin/node"));
            // A genuine user Node without npm still gets the complete managed
            // npm fallback, and that fallback must not install into its cache.
            let tool = crate::resolve_existing_tool(ToolId::Node).await.unwrap();
            let user_bin = root.join("user-node-only"); fs::create_dir(&user_bin).unwrap();
            fs::copy(&tool.executable_path, user_bin.join("node")).unwrap();
            let configured = HashMap::from([
                ("PATH".into(), format!("{}:/usr/bin:/bin", user_bin.display())),
                ("HOME".into(), root.join("home").to_string_lossy().into_owned()),
                ("NPM_CONFIG_USERCONFIG".into(), root.join("home/.npmrc").to_string_lossy().into_owned()),
            ]);
            let launch = crate::prepare_process_launch("npm", &["--version".into()], &root, &configured).await.unwrap();
            assert_eq!(launch.environment.get("NPM_CONFIG_PREFIX").map(String::as_str), crate::user_cli_root().unwrap().to_str());
            assert!(launch.environment["PATH"].starts_with(&user_bin.to_string_lossy().into_owned()));
        } else {
            execute(&root, "node", &["--version".into()]).await;
        }
        let args: Vec<String> = ["", "含 空格", "double\"quote", "single'quote", "$(touch SHOULD_NOT_EXIST);&|<>", "line\nbreak"].into_iter().map(str::to_owned).collect();
        let value = execute(&root, "nomifun-local-fixture", &args).await;
        assert_eq!(value["argv"], serde_json::json!(args)); assert_eq!(value["home"], root.join("home").to_string_lossy().as_ref());
        let mut npx = vec!["--no-install".into(), "--offline".into(), "nomifun-local-fixture".into()]; npx.extend(args.clone());
        let value = execute(&root, "npx", &npx).await; assert_eq!(value["argv"], serde_json::json!(args));
        if phase == "install" {
            // Repair a removed managed payload in this same running host. User
            // tools remain outside the cache and need no reinstall or restart.
            let tool = crate::resolve_existing_tool(ToolId::Node).await.unwrap();
            fs::remove_dir_all(tool.executable_path.parent().unwrap().parent().unwrap()).unwrap();
            assert!(crate::resolve_existing_tool(ToolId::Node).await.is_err());
            let value = execute(&root, "nomifun-local-fixture", &args).await;
            assert_eq!(value["argv"], serde_json::json!(args));
            execute(&root, "npm", &["--version".into()]).await;
        }
        assert!(!root.join("SHOULD_NOT_EXIST").exists());
        assert!(!crate::user_cli_root().unwrap().starts_with(crate::runtime_root().unwrap()));
        if phase == "restart" {
            execute(&root, "npm", &["uninstall".into(), "--global".into(), "--offline".into(), "nomifun-local-fixture".into(), "--no-audit".into(), "--no-fund".into()]).await;
            assert!(!crate::user_cli_root().unwrap().join("bin/nomifun-local-fixture").exists());
        }
        return;
    }
    let spec = specification(ToolId::Node);
    let asset = spec.asset(env!("NOMIFUN_RUNTIME_TARGET"), "default").unwrap();
    let cargo_home = std::env::var_os("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|| dirs::home_dir().unwrap().join(".cargo"));
    let archive = cargo_home.join("nomifun-component-cache/node").join(&asset.sha256).join("approved.asset");
    assert!(archive.is_file(), "build.rs must retain its verified official archive");
    let temporary = tempfile::tempdir().unwrap(); let root = dunce::canonicalize(temporary.path()).unwrap(); fs::create_dir_all(root.join("home")).unwrap(); archive_fixture(&root); seed_verified_archive(&root, &archive);
    for phase in ["install", "retained", "restart"] {
        if phase == "retained" {
            // Cache replacement must not remove already installed user tools.
            fs::remove_dir_all(root.join("data/runtime")).unwrap(); seed_verified_archive(&root, &archive);
        }
        let mut child = nomi_process_runtime::ChildProcessBuilder::clean_cli(std::env::current_exe().unwrap());
        child.args(["--ignored", "--exact", TEST, "--nocapture"]).env(CHILD_ROOT, &root).env(CHILD_PHASE, phase)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin").env("HOME", root.join("home"))
            .env("USERPROFILE", root.join("home")).env("APPDATA", root.join("appdata"))
            .env("LOCALAPPDATA", root.join("localappdata")).env("XDG_DATA_HOME", root.join("xdg-data"))
            .env_remove("NOMIFUN_NODE_PATH").env_remove("NOMIFUN_BUN_PATH").env_remove("NPM_CONFIG_PREFIX")
            .env_remove("npm_config_prefix").env_remove("NODE_OPTIONS");
        let output = child.output().await.unwrap();
        assert!(output.status.success(), "phase {phase}: stdout={} stderr={}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
}
