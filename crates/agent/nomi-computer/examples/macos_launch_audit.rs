//! Explicit bare-name LaunchServices handoff against an independently prepared
//! audit App. This example does not register Apps or query/request permissions.

#[cfg(target_os = "macos")]
fn main() {
    match audit() {
        Ok(report) => println!("{report}"),
        Err(error) => {
            println!("{}", serde_json::json!({
                "passed": false,
                "scope": "macos_computer_launch_audit",
                "error": error,
            }));
            std::process::exit(1);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("{}", serde_json::json!({
        "passed": false,
        "scope": "macos_computer_launch_audit",
        "error": "This native launch audit requires macOS.",
    }));
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn audit() -> Result<serde_json::Value, String> {
    const PREFIX: &str = "NomiFun Launch Audit ";
    const MAX_SUFFIX_BYTES: usize = 64;

    let mut args = std::env::args_os().skip(1);
    let name = args.next().ok_or("exactly one audit App name is required")?
        .into_string().map_err(|_| "audit App name must be valid UTF-8")?;
    if args.next().is_some() {
        return Err("exactly one audit App name is required".to_owned());
    }
    let suffix = name.strip_prefix(PREFIX)
        .ok_or("only independently prepared NomiFun Launch Audit Apps are accepted")?;
    if suffix.is_empty() || suffix.len() > MAX_SUFFIX_BYTES
        || !suffix.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("audit name suffix must contain 1–64 ASCII letters, digits or hyphens".to_owned());
    }

    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    let handoff = runtime.block_on(nomi_computer::launch::launch(&name, None))?;
    Ok(serde_json::json!({
        "passed": true,
        "scope": "macos_computer_launch_audit",
        "target": name,
        "handoff": handoff,
        "actual_application_start_verified": false,
    }))
}
