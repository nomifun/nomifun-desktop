//! Opt-in Linux component lifecycle proof using the real shared runtime, MCP
//! owner, canonical Session store and Agent-only database reset. This is not a
//! native desktop App/LLM end-to-end or retired-generation upgrade assertion.
#![cfg(target_os = "linux")]

use nomifun_agent_contracts::{
    AgentBindingValue, AgentPresetId, AgentSessionId, AgentSessionLiveRecord, AgentSessionMetadata,
    DigestHex, OperationId, PresetRevisionRef, PrincipalRef, ResolvedSnapshotId,
    ResolvedSnapshotRef, StrictJsonValue,
};
use nomifun_agent_session::{AgentSessionStore, CreateSessionRequest};
use nomifun_mcp::{
    AnonymousMcpCredentialAuthority, MCP_CONNECT_OPERATION, MCP_INVOKE_OPERATION,
    MCP_SERVER_RESOURCE_KIND, McpOwner, McpServerBinding, McpServerTransport, McpToolBinding,
    McpToolInvocationRequest,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};

const TEST: &str = "linux_managed_node_mcp_reset_and_new_processes";
const ROOT: &str = "NOMIFUN_LINUX_NODE_LIFECYCLE_ROOT";
const PHASE: &str = "NOMIFUN_LINUX_NODE_LIFECYCLE_PHASE";
const CLI: &str = "nomifun-linux-node-lifecycle-fixture";

fn digest_file(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut bytes = [0; 64 * 1024];
    loop {
        let n = file.read(&mut bytes).unwrap();
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    format!("{:x}", hash.finalize())
}

fn cli_identity(root: &Path) -> Value {
    let prefix = root.join("xdg-data/nomifun-user-tools/npm");
    let link = prefix.join("bin").join(CLI);
    let path = fs::canonicalize(&link).unwrap();
    assert!(path.starts_with(&prefix));
    let info = fs::metadata(&path).unwrap();
    json!({"path":path,"link":link,"uid":info.uid(),"dev":info.dev(),"inode":info.ino(),"sha256":digest_file(&path)})
}

fn configured(root: &Path) -> HashMap<String, String> {
    HashMap::from([
        (
            "PATH".into(),
            root.join("shell-only").to_string_lossy().into_owned(),
        ),
        (
            "NPM_CONFIG_USERCONFIG".into(),
            root.join("npmrc-empty").to_string_lossy().into_owned(),
        ),
        (
            "NPM_CONFIG_CACHE".into(),
            root.join("npm-cache").to_string_lossy().into_owned(),
        ),
        (
            "NPM_CONFIG_REGISTRY".into(),
            "http://127.0.0.1:9/never-network".into(),
        ),
        ("NPM_CONFIG_OFFLINE".into(), "true".into()),
        (
            "NOMIFUN_FIXTURE_PROOF".into(),
            root.join("postinstall.json").to_string_lossy().into_owned(),
        ),
    ])
}

async fn execute(root: &Path, command: &str, args: &[String]) -> Value {
    let launch = nomifun_runtime::prepare_process_launch(command, args, root, &configured(root))
        .await
        .unwrap();
    assert_eq!(launch.environment["HOME"], std::env::var("HOME").unwrap());
    let mut process = nomi_process_runtime::ChildProcessBuilder::clean_cli(&launch.program);
    process
        .args(&launch.args)
        .env_clear()
        .envs(&launch.environment)
        .current_dir(root);
    let output = tokio::time::timeout(Duration::from_secs(60), process.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "{command}: {} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line).ok())
        .unwrap_or(Value::Null)
}

fn literal_arguments() -> Vec<String> {
    [
        "",
        "中文 space",
        "double\"quote",
        "single'quote",
        "$(touch MUST_NOT_EXIST);&|<>",
        "line\nbreak",
    ]
    .map(str::to_owned)
    .into()
}

fn validate_process(root: &Path, value: &Value, args: &[String]) {
    assert_eq!(value["argv"], json!(args));
    assert_eq!(value["home"], std::env::var("HOME").unwrap());
    assert_eq!(value["cwd"], root.to_string_lossy().as_ref());
    assert_eq!(value["uid"], fs::metadata(root).unwrap().uid());
    assert!(Path::new(value["execPath"].as_str().unwrap()).starts_with(root.join("data/runtime")));
    assert!(!root.join("MUST_NOT_EXIST").exists());
}

async fn invoke_mcp(root: &Path, phase: &str) -> Value {
    let args = [vec!["--mcp".into()], literal_arguments()].concat();
    let schema = json!({"type":"object","properties":{"phase":{"type":"string"}},"required":["phase"],"additionalProperties":false});
    let owner = McpOwner::new(
        Arc::new(AnonymousMcpCredentialAuthority),
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
    );
    let result = owner
        .invoke(McpToolInvocationRequest {
            principal_kind: "user".into(),
            principal_id: "node-lifecycle-owner".into(),
            operation_id: format!("node-mcp-{phase}"),
            server: McpServerBinding {
                server_id: "node-lifecycle-server".into(),
                server_owner_id: "node-lifecycle-owner".into(),
                enabled: true,
                connection_config_ref: "node-lifecycle-config".into(),
                resource_binding_id: "node-lifecycle-resource".into(),
                resource_kind: MCP_SERVER_RESOURCE_KIND.into(),
                resource_id: "node-lifecycle-server".into(),
                resource_owner_id: "node-lifecycle-owner".into(),
                granted_operations: BTreeSet::from([
                    MCP_CONNECT_OPERATION.into(),
                    MCP_INVOKE_OPERATION.into(),
                ]),
                resource_connection_config_ref: Some("node-lifecycle-config".into()),
                transport: McpServerTransport::Stdio {
                    command: CLI.into(),
                    args: args.clone(),
                    env: configured(root),
                },
            },
            tool: McpToolBinding::new(
                "node-lifecycle-server",
                "fixture.echo",
                "a".repeat(64),
                schema,
                "echo",
            )
            .unwrap(),
            arguments: json!({"phase":phase}),
        })
        .await
        .unwrap();
    let value: Value =
        serde_json::from_str(result.result["content"][0]["text"].as_str().unwrap()).unwrap();
    validate_process(root, &value, &args);
    assert_eq!(value["arguments"], json!({"phase":phase}));
    let pid = value["pid"].as_u64().unwrap();
    assert!(
        pid > 1 && !Path::new(&format!("/proc/{pid}")).exists(),
        "successful MCP result requires actual stdio process-tree cleanup"
    );
    value
}

async fn canonical_reset(root: &Path) -> Value {
    let database = nomifun_db::init_database(&root.join("data/node-lifecycle.db"))
        .await
        .unwrap();
    let owner_id = nomifun_db::installation_owner_id(database.pool())
        .await
        .unwrap();
    let owner = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: owner_id.clone(),
    };
    let store = AgentSessionStore::from_pool(database.pool().clone())
        .await
        .unwrap();
    let id: AgentSessionId = "0190f5fe-7c00-7a00-8000-000000000701".into();
    let created = store
        .create_session(CreateSessionRequest::new(
            AgentSessionLiveRecord {
                agent_session_id: id.clone(),
                owner_ref: owner,
                metadata: AgentSessionMetadata {
                    title: Some("Node reset boundary fixture".into()),
                    purpose: Default::default(),
                    app_project_id: None,
                    companion: None,
                    archived: false,
                    pinned: false,
                    reasoning_effort: None,
                },
                agent_binding: AgentBindingValue {
                    preset_revision_ref: PresetRevisionRef {
                        preset_id: AgentPresetId::from("node-lifecycle-fixture"),
                        revision: 1,
                        revision_digest: DigestHex::from("a".repeat(64)),
                    },
                    resolved_snapshot_ref: ResolvedSnapshotRef {
                        snapshot_id: ResolvedSnapshotId::from("node-lifecycle-snapshot"),
                        snapshot_digest: DigestHex::from("b".repeat(64)),
                    },
                    typed_resource_bindings: vec![],
                    binding_version: 1,
                },
                remote_binding_provenance: None,
                parent_session_id: None,
                fork_base_payload_id: None,
                next_seq: 1,
            },
            1_792_200_000_000,
            "node-lifecycle-create",
            "session-api".into(),
            "node-lifecycle-create".into(),
            "node-lifecycle-session".into(),
        ))
        .await
        .unwrap();
    store
        .append_event(&nomifun_agent_contracts::SessionEventAppend {
            agent_session_id: id.clone(),
            event_id: "node-lifecycle-ready-event".into(),
            producer_id: "runtime-supervisor".into(),
            idempotency_key: "node-lifecycle-ready".into(),
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: nomifun_agent_contracts::SessionEventKind("session/ready".into()),
                kind_version: 1,
                correlation_id: "node-lifecycle-session".into(),
                causation_event_id: Some(created.opening_ack.event_id),
                payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                    StrictJsonValue(json!({})),
                ),
            },
        })
        .await
        .unwrap();
    for suffix in ["first", "successor"] {
        let operation: OperationId = format!("node-lifecycle-{suffix}").into();
        store
            .start_turn(
                &id,
                "session-api".into(),
                format!("input-{suffix}").into(),
                operation.clone(),
                StrictJsonValue(json!({"content":format!("Canonical fixture {suffix}")})),
            )
            .await
            .unwrap();
        let value = execute(root, CLI, &literal_arguments()).await;
        validate_process(root, &value, &literal_arguments());
        store
            .cancel_exact_turn(
                &id,
                &operation,
                format!("cancel-{suffix}").into(),
                "session-api".into(),
            )
            .await
            .unwrap();
    }
    let counts_before: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_sessions),(SELECT COUNT(*) FROM agent_turns),(SELECT COUNT(*) FROM agent_events)").fetch_one(database.pool()).await.unwrap();
    assert_eq!(counts_before.0, 1);
    assert_eq!(counts_before.1, 2);
    assert!(counts_before.2 >= 6);
    let identity_before = cli_identity(root);
    let reset = nomifun_db::reset_agent_data(database.pool()).await.unwrap();
    assert_eq!(reset.deleted_rows["agent_sessions"], 1);
    assert_eq!(reset.deleted_rows["agent_turns"], 2);
    let counts_after: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_sessions),(SELECT COUNT(*) FROM agent_turns),(SELECT COUNT(*) FROM agent_events)").fetch_one(database.pool()).await.unwrap();
    assert_eq!(counts_after, (0, 0, 0));
    assert_eq!(
        nomifun_db::installation_owner_id(database.pool())
            .await
            .unwrap(),
        owner_id
    );
    assert_eq!(cli_identity(root), identity_before);
    let cli_after = execute(root, CLI, &literal_arguments()).await;
    validate_process(root, &cli_after, &literal_arguments());
    let mcp_after = invoke_mcp(root, "after-agent-data-reset").await;
    json!({"scope":"public canonical Store API component + public Agent-only reset; no App execution, native Runtime journal or tool Effect attribution claim","counts_before":counts_before,"counts_after":counts_after,"deleted_rows":reset.deleted_rows,"preserved_rows":reset.preserved_rows,"installation_owner_preserved":true,"cli_identity_before":identity_before,"cli_identity_after":cli_identity(root),"cli_after":cli_after,"mcp_after":mcp_after})
}

fn seed_archive(root: &Path, official: &Path, version: &str, executable_sha: &str) {
    let destination = root
        .join("data/runtime")
        .join(format!("node-{version}-{executable_sha}"));
    fs::create_dir_all(&destination).unwrap();
    fs::copy(official, destination.join("approved.asset")).unwrap();
}

#[tokio::test]
#[ignore = "opt-in real verified Node/offline npm/MCP/Agent-only reset; private XDG, preserved real HOME"]
async fn linux_managed_node_mcp_reset_and_new_processes() {
    if let Some(root) = std::env::var_os(ROOT) {
        let root = fs::canonicalize(root).unwrap();
        assert_eq!(fs::metadata(&root).unwrap().mode() & 0o077, 0);
        assert_eq!(
            PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap()),
            root.join("xdg-data")
        );
        assert_eq!(
            std::env::var("PATH").unwrap(),
            root.join("shell-only").to_string_lossy()
        );
        assert!(root.join("shell-only/sh").exists());
        assert!(!root.join("shell-only/node").exists());
        nomifun_runtime::init(root.join("data"));
        let phase = std::env::var(PHASE).unwrap();
        if phase == "install-reset" {
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
                    "--update-notifier=false".into(),
                    root.join("fixture.tgz").to_string_lossy().into_owned(),
                ],
            )
            .await;
            let postinstall: Value =
                serde_json::from_slice(&fs::read(root.join("postinstall.json")).unwrap()).unwrap();
            assert_eq!(postinstall["home"], std::env::var("HOME").unwrap());
            assert!(
                Path::new(postinstall["execPath"].as_str().unwrap())
                    .starts_with(root.join("data/runtime"))
            );
            let before = execute(&root, CLI, &literal_arguments()).await;
            validate_process(&root, &before, &literal_arguments());
            let mcp_before = invoke_mcp(&root, "before-agent-data-reset").await;
            let reset = canonical_reset(&root).await;
            fs::write(
                root.join("installed-identity.json"),
                serde_json::to_vec(&cli_identity(&root)).unwrap(),
            )
            .unwrap();
            fs::write(root.join("install-reset.json"),serde_json::to_vec_pretty(&json!({"phase":phase,"real_home":std::env::var("HOME").unwrap(),"postinstall":postinstall,"cli_before":before,"mcp_before":mcp_before,"canonical_reset":reset})).unwrap()).unwrap();
        } else {
            let expected: Value =
                serde_json::from_slice(&fs::read(root.join("installed-identity.json")).unwrap())
                    .unwrap();
            assert_eq!(cli_identity(&root), expected);
            let value = execute(&root, CLI, &literal_arguments()).await;
            validate_process(&root, &value, &literal_arguments());
            let mcp = invoke_mcp(&root, &phase).await;
            assert_eq!(cli_identity(&root), expected);
            fs::write(
                root.join(format!("{phase}.json")),
                serde_json::to_vec_pretty(
                    &json!({"phase":phase,"cli_identity":expected,"cli":value,"mcp":mcp}),
                )
                .unwrap(),
            )
            .unwrap();
            if phase == "restart-uninstall" {
                execute(
                    &root,
                    "npm",
                    &[
                        "uninstall".into(),
                        "--global".into(),
                        "--offline".into(),
                        CLI.into(),
                        "--no-audit".into(),
                        "--no-fund".into(),
                    ],
                )
                .await;
                assert!(
                    !root
                        .join("xdg-data/nomifun-user-tools/npm/bin")
                        .join(CLI)
                        .exists()
                );
            }
        }
        return;
    }
    let actual_home = std::env::var_os("HOME").expect("login HOME is required");
    assert!(Path::new(&actual_home).is_absolute());
    let manifest: Value =
        serde_json::from_str(include_str!("../../nomifun-runtime/components.json")).unwrap();
    let node = manifest["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["id"] == "node")
        .unwrap();
    let target = match std::env::consts::ARCH {
        "x86_64" => "x86_64-unknown-linux-gnu",
        "aarch64" => "aarch64-unknown-linux-gnu",
        _ => panic!("unsupported official Node target"),
    };
    let asset = node["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["target"] == target && asset["variant"] == "default")
        .unwrap();
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&actual_home).join(".cargo"));
    let official = cargo_home
        .join("nomifun-component-cache/node")
        .join(asset["sha256"].as_str().unwrap())
        .join("approved.asset");
    assert!(
        official.is_file(),
        "use the already verified cached official archive; this opt-in test never downloads"
    );
    assert_eq!(digest_file(&official), asset["sha256"].as_str().unwrap());
    let version = node["version"].as_str().unwrap();
    let platform_arch = if target.starts_with("x86_64") {
        "x64"
    } else {
        "arm64"
    };
    let mut tar = Command::new("/usr/bin/tar")
        .args([
            "-xOf",
            official.to_str().unwrap(),
            &format!("node-v{version}-linux-{platform_arch}/bin/node"),
        ])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut bytes = [0; 64 * 1024];
    let mut hash = Sha256::new();
    let mut stdout = tar.stdout.take().unwrap();
    loop {
        let n = stdout.read(&mut bytes).unwrap();
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    assert!(tar.wait().unwrap().success());
    let executable_sha = format!("{:x}", hash.finalize());
    let temporary = tempfile::Builder::new()
        .prefix("nomifun-linux-node-lifecycle-")
        .tempdir()
        .unwrap();
    let root = fs::canonicalize(temporary.path()).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        fs::metadata(&root).unwrap().uid() > 0,
        "run as the ordinary login user"
    );
    for directory in [
        "data",
        "xdg-data",
        "xdg-config",
        "xdg-cache",
        "shell-only",
        "source/package",
    ] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    fs::write(root.join("npmrc-empty"), b"").unwrap();
    std::os::unix::fs::symlink("/bin/sh", root.join("shell-only/sh")).unwrap();
    fs::write(root.join("source/package/package.json"),json!({"name":CLI,"version":"1.0.0","bin":{"nomifun-linux-node-lifecycle-fixture":"fixture.cjs"},"scripts":{"postinstall":"node postinstall.cjs"}}).to_string()).unwrap();
    fs::write(root.join("source/package/postinstall.cjs"),"require('node:fs').writeFileSync(process.env.NOMIFUN_FIXTURE_PROOF,JSON.stringify({home:process.env.HOME,execPath:process.execPath}));\n").unwrap();
    fs::write(root.join("source/package/fixture.cjs"),r#"#!/usr/bin/env node
const readline=require('node:readline');
const state=()=>({argv:process.argv.slice(2),home:process.env.HOME,uid:process.getuid(),cwd:process.cwd(),execPath:process.execPath,pid:process.pid});
if(process.argv[2]!=='--mcp'){console.log(JSON.stringify(state()));}else{
 const schema={type:'object',properties:{phase:{type:'string'}},required:['phase'],additionalProperties:false};
 const input=readline.createInterface({input:process.stdin});input.on('line',line=>{
  const request=JSON.parse(line);if(request.id===undefined)return;let result;
  if(request.method==='initialize')result={protocolVersion:'2025-03-26',capabilities:{tools:{}},serverInfo:{name:'private-node-lifecycle',version:'1'}};
  else if(request.method==='tools/list')result={tools:[{name:'echo',description:'Private lifecycle fixture',inputSchema:schema}]};
  else if(request.method==='tools/call')result={content:[{type:'text',text:JSON.stringify({...state(),arguments:request.params.arguments})}]};
  else throw Error('Unexpected fixture method');process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:request.id,result})+'\n');
 });
}
"#).unwrap();
    fs::set_permissions(
        root.join("source/package/fixture.cjs"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(
        Command::new("/usr/bin/tar")
            .args([
                "-czf",
                root.join("fixture.tgz").to_str().unwrap(),
                "-C",
                root.join("source").to_str().unwrap(),
                "package"
            ])
            .status()
            .unwrap()
            .success()
    );
    seed_archive(&root, &official, version, &executable_sha);
    for phase in ["install-reset", "cache-replaced", "restart-uninstall"] {
        if phase == "cache-replaced" {
            fs::remove_dir_all(root.join("data/runtime")).unwrap();
            seed_archive(&root, &official, version, &executable_sha);
        }
        let mut process =
            nomi_process_runtime::ChildProcessBuilder::clean_cli(std::env::current_exe().unwrap());
        process
            .current_dir(&root)
            .args(["--ignored", "--exact", TEST, "--nocapture"])
            .env(ROOT, &root)
            .env(PHASE, phase)
            .env("PATH", root.join("shell-only"))
            .env("XDG_DATA_HOME", root.join("xdg-data"))
            .env("XDG_CONFIG_HOME", root.join("xdg-config"))
            .env("XDG_CACHE_HOME", root.join("xdg-cache"))
            .env_remove("NOMIFUN_NODE_PATH")
            .env_remove("NOMIFUN_BUN_PATH")
            .env_remove("NPM_CONFIG_PREFIX")
            .env_remove("npm_config_prefix")
            .env_remove("NODE_OPTIONS");
        let output = tokio::time::timeout(Duration::from_secs(180), process.output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            output.status.success(),
            "phase {phase}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let phases: Vec<Value> = ["install-reset", "cache-replaced", "restart-uninstall"]
        .iter()
        .map(|phase| {
            serde_json::from_slice(&fs::read(root.join(format!("{phase}.json"))).unwrap()).unwrap()
        })
        .collect();
    let report = json!({"schema":"nomifun.linux-node-lifecycle-component.v1","status":"passed","evidence_scope":"Real Linux component processes/shared runtime/MCP owner/canonical Store API/Agent-only reset. No native desktop App execution or MCP Capability/Effect attribution. Two Store Turns are accepted/cancelled fixtures; direct CLI execution is not attributed to them.","uid":fs::metadata(&root).unwrap().uid(),"home":actual_home.to_str().expect("login HOME must fit the UTF-8 process contract"),"fixture":root,"official_archive_sha256":asset["sha256"],"node_executable_sha256":executable_sha,"no_download":true,"phases":phases,"uninstalled":true});
    temporary.close().unwrap();
    assert!(!root.exists());
    let mut report = report;
    report["owned_fixture_removed"] = json!(true);
    if let Some(path) = std::env::var_os("NOMIFUN_LINUX_NODE_LIFECYCLE_REPORT") {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap()
            .write_all(serde_json::to_string_pretty(&report).unwrap().as_bytes())
            .unwrap();
    }
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}

use std::io::Write;
