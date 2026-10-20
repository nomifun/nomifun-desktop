//! Ordinary acceptance against a task-owned WinForms calculator and editor.
//! Uses only exact-window AX tokens; no screenshots, clipboard or pixel input.
use nomifun_agent_kernel::InvocationControl;
use nomifun_computer::{Computer, WorkerConfig, result};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};

async fn wait(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    Ok(())
}
struct Calls<'a> {
    computer: &'a Computer,
    authorized: BTreeSet<String>,
    root: &'a Path,
    sequence: usize,
    evidence: Vec<Value>,
}
impl Calls<'_> {
    async fn call(&mut self, name: &str, args: Value) -> Result<Value, Box<dyn std::error::Error>> {
        self.sequence += 1;
        let reply = self
            .computer
            .call(
                "owned-desktop-test",
                "Owned desktop test",
                name,
                args,
                &self.authorized,
                false,
                InvocationControl::default(),
            )
            .await?;
        let normalized = result::normalize(&reply.result, false);
        std::fs::write(
            self.root.join(format!("call-{:02}.json", self.sequence)),
            serde_json::to_vec_pretty(&reply.result)?,
        )?;
        self.evidence.push(json!({"action":name,"completion":reply.completion,"effect":normalized.effect,"is_error":normalized.is_error,"foreground":reply.foreground}));
        if normalized.is_error {
            return Err(format!("owned-window action failed: {name}").into());
        }
        if normalized
            .parts
            .iter()
            .any(|part| matches!(part, result::ResultPart::Image { .. }))
        {
            return Err("unexpected image result".into());
        }
        Ok(normalized.structured_content.unwrap_or(reply.result))
    }
    async fn snapshot(
        &mut self,
        pid: u64,
        window: u64,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        self.call("computer/get_window_state",json!({"pid":pid,"window_id":window,"include_screenshot":false,"tree_format":"elements","max_elements":100,"timeout_ms":3000})).await
    }
    async fn token(
        &mut self,
        pid: u64,
        window: u64,
        label: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let state = self.snapshot(pid, window).await?;
        state
            .get("elements")
            .and_then(Value::as_array)
            .and_then(|elements| elements.iter().find(|item| item["label"] == label))
            .and_then(|item| item["element_token"].as_str())
            .map(str::to_owned)
            .ok_or_else(|| format!("owned AX control missing: {label}").into())
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if !cfg!(windows) {
        return Err("Windows fixture only".into());
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("pass worker, dedicated fixture executable and owned root".into());
    }
    let root = PathBuf::from(&args[2]);
    let computer = Computer::new(WorkerConfig::new(
        PathBuf::from(&args[0]),
        root.join("host-data"),
    ));
    let mut calls = Calls {
        computer: &computer,
        authorized: [
            "computer/launch_app",
            "computer/get_window_state",
            "computer/click",
            "computer/type_text",
        ]
        .map(str::to_owned)
        .into_iter()
        .collect(),
        root: &root,
        sequence: 0,
        evidence: Vec::new(),
    };
    let outcome=async{
        calls.call("computer/launch_app",json!({"launch_path":PathBuf::from(&args[1]),"additional_arguments":["functional",root],"start_minimized":false})).await?;
        wait(&root.join("ready.json")).await?;
        let ready:Value=serde_json::from_slice(&std::fs::read(root.join("ready.json"))?)?;
        let pid=ready["pid"].as_u64().ok_or("owned fixture PID missing")?;
        let window=ready["window"].as_str().ok_or("owned HWND missing")?.parse::<u64>()?;
        for label in ["Owned two","Owned add","Owned three","Owned equals"]{
            let token=calls.token(pid,window,label).await?;
            calls.call("computer/click",json!({"element_token":token,"delivery_mode":"background"})).await?;
        }
        wait(&root.join("calculation.json")).await?;
        let calculation:Value=serde_json::from_slice(&std::fs::read(root.join("calculation.json"))?)?;
        if calculation["result"]!="5"{return Err::<Value,Box<dyn std::error::Error>>("calculator result was not 5".into());}
        let state=calls.snapshot(pid,window).await?;
        if !state["elements"].as_array().is_some_and(|elements|elements.iter().any(|item|item["label"]=="Owned calculation"&&item["value"]=="5")){return Err("AX did not verify calculator display 5".into());}
        let text="NomiFun Computer Use owned test\r\nCalculator 2 + 3 = 5; 中文文本。";
        let token=calls.token(pid,window,"Owned editor").await?;
        calls.call("computer/type_text",json!({"element_token":token,"text":text,"delivery_mode":"background"})).await?;
        let token=calls.token(pid,window,"Save owned test text").await?;
        calls.call("computer/click",json!({"element_token":token,"delivery_mode":"background"})).await?;
        wait(&root.join("saved.json")).await?;
        if std::fs::read_to_string(root.join("saved.txt"))?!=text{return Err("saved owned text differed".into());}
        Ok(json!({"ok":true,"calculator":"2+3=5","text_saved":true,"text_characters":text.chars().count(),"app_pid":pid,"worker_pid":computer.status().process_id,"worker_version":computer.status().worker_version,"dpi":ready["dpi"],"screenshots":0,"clipboard_calls":0,"pixel_calls":0,"foreground_calls":calls.evidence.iter().filter(|entry|entry["foreground"]==true).count(),"calls":calls.evidence}))
    }.await;
    let closed = computer.close_session("owned-desktop-test").await;
    let shutdown = computer.shutdown().await;
    std::fs::write(root.join("close.request"), b"close only this fixture")?;
    let app_close = if root.join("ready.json").exists() {
        wait(&root.join("exit.json")).await
    } else {
        Ok(())
    };
    let report = match outcome {
        Ok(report) => report,
        Err(error) => {
            std::fs::write(
                root.join("functional-error.json"),
                serde_json::to_vec_pretty(
                    &json!({"error":error.to_string(),"calls":calls.evidence}),
                )?,
            )?;
            return Err(error);
        }
    };
    closed?;
    shutdown?;
    app_close?;
    std::fs::write(
        root.join("functional-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", report);
    Ok(())
}
