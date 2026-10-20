//! Ordinary Win32 launch/retirement acceptance. The observer and GUI are owned
//! test fixtures; no input injection, screenshot or forced process stop occurs.
use nomifun_agent_kernel::InvocationControl;
use nomifun_computer::{Computer, WorkerConfig, result};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};

fn publish(root: &Path, name: &str, value: Value) -> std::io::Result<()> {
    std::fs::write(root.join(name), serde_json::to_vec_pretty(&value)?)
}
async fn wait_file(path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(45), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if !cfg!(windows) {
        return Err("this acceptance fixture is Windows-only".into());
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !matches!(args.len(), 3 | 4) {
        return Err(
            "pass worker binary, fixture executable, owned test root and optional direct|shell"
                .into(),
        );
    }
    let direct = match args
        .get(3)
        .and_then(|value| value.to_str())
        .unwrap_or("direct")
    {
        "direct" => true,
        "shell" => false,
        _ => return Err("launch mode must be direct or shell".into()),
    };
    let root = PathBuf::from(&args[2]);
    let mut config = WorkerConfig::new(PathBuf::from(&args[0]), root.join("host-data"));
    // Exercise the normal idle-retirement path without waiting five minutes.
    // This changes only this fixture owner; no user/application setting changes.
    config.idle_timeout = Duration::from_millis(250);
    let computer = Computer::new(config);
    let authorized: BTreeSet<_> = ["computer/launch_app".to_owned()].into_iter().collect();
    let outcome=async{
        let launched=computer.call("owned-launch-test","Owned launch test","computer/launch_app",json!({"launch_path":PathBuf::from(&args[1]),"additional_arguments":["gui",root],"start_minimized":direct}),&authorized,false,InvocationControl::default()).await?;
        let result=result::normalize(&launched.result,false);
        if result.is_error{return Err::<(),Box<dyn std::error::Error>>("fixture launch was refused".into());}
        wait_file(root.join("ready.json")).await?;
        publish(&root,"launched.json",json!({"host_pid":std::process::id(),"worker_pid":computer.status().process_id,"generation":launched.generation,"worker_version":computer.status().worker_version,"idle_timeout_ms":250,"launch_mode":if direct{"direct"}else{"shell"},"forced_stop":false}))?;
        wait_file(root.join("observer-ready")).await?;
        computer.close_session("owned-launch-test").await?;
        publish(&root,"session-closed.json",json!({"worker_running":computer.status().running}))?;
        tokio::time::timeout(Duration::from_secs(15),async{while computer.status().running{tokio::time::sleep(Duration::from_millis(10)).await;}}).await?;
        publish(&root,"retired.json",json!({"worker_running":computer.status().running,"phase":computer.status().phase}))?;
        Ok(())
    }.await;
    let shutdown = computer.shutdown().await;
    if let Err(error) = outcome {
        publish(&root, "host-error.json", json!({"error":error.to_string()}))?;
        return Err(error);
    }
    shutdown?;
    publish(
        &root,
        "finished.json",
        json!({"ok":true,"worker_running":computer.status().running,"desktop_input_calls":0,"screenshots":0,"forced_stop":false}),
    )?;
    Ok(())
}
