use super::*;

const FIXTURE_MODE: &str = "NOMIFUN_PROXY_COMMAND_FIXTURE";
const CHILD_IDENTITY_PATH: &str = "NOMIFUN_PROXY_COMMAND_CHILD_IDENTITY";

fn fixture_command(mode: &str) -> std::process::Command {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "proxy::command_tests::proxy_command_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env(FIXTURE_MODE, mode);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

fn managed_fixture_command(mode: &str) -> ChildProcessBuilder {
    let fixture = fixture_command(mode);
    let mut command = ChildProcessBuilder::new(fixture.get_program());
    command.args(fixture.get_args()).env(FIXTURE_MODE, mode);
    command
}

#[test]
#[ignore = "subprocess-only fixture, invoked with an explicit mode"]
fn proxy_command_fixture() {
    match std::env::var(FIXTURE_MODE).as_deref() {
        Ok("hold_stdout") => std::thread::sleep(Duration::from_millis(1500)),
        Ok("descendant") => {
            let child = fixture_command("hold_stdout")
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            if let Some(path) = std::env::var_os(CHILD_IDENTITY_PATH) {
                let identity = nomi_process_runtime::probe_process_identity(child.id())
                    .unwrap()
                    .unwrap();
                std::fs::write(
                    path,
                    format!("{}:{}", identity.pid, identity.platform_start_key),
                )
                .unwrap();
            }
            println!("proxy parent exited");
            std::process::exit(0);
        }
        Ok("oversized") => {
            use std::io::Write;
            let _ = std::io::stdout().write_all(&vec![b'x'; 128 * 1024]);
        }
        Ok("failure") => std::process::exit(7),
        Ok("success") => println!("proxy fixture ready"),
        _ => {}
    }
}

#[test]
fn exited_parent_with_an_inherited_stdout_cannot_bypass_timeout() {
    let temp = tempfile::TempDir::new().unwrap();
    let identity_path = temp.path().join("child-identity");
    let mut command = managed_fixture_command("descendant");
    command.env(CHILD_IDENTITY_PATH, &identity_path);
    let started = Instant::now();
    let result = command_stdout_with_timeout(command, Duration::from_millis(200));
    let recorded = std::fs::read_to_string(identity_path).unwrap();
    let (pid, start) = recorded.split_once(':').unwrap();
    let pid: u32 = pid.parse().unwrap();
    let start: u64 = start.parse().unwrap();
    while nomi_process_runtime::probe_process_identity(pid)
        .unwrap()
        .is_some_and(|live| live.platform_start_key == start)
    {
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "descendant survived cleanup before its fixture fallback"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "stdout read outlived its deadline: {:?}, output present={}",
        started.elapsed(),
        result.is_some()
    );
}

#[test]
fn command_output_requires_success_and_a_bounded_body() {
    let output =
        command_stdout_with_timeout(managed_fixture_command("success"), Duration::from_secs(2))
            .unwrap();
    assert!(output.contains("proxy fixture ready"));
    for mode in ["failure", "oversized"] {
        assert!(
            command_stdout_with_timeout(managed_fixture_command(mode), Duration::from_millis(200),)
                .is_none(),
            "invalid {mode} output was accepted"
        );
    }
}

#[tokio::test]
async fn synchronous_probe_can_run_inside_an_existing_runtime() {
    let output =
        command_stdout_with_timeout(managed_fixture_command("success"), Duration::from_secs(2))
            .unwrap();
    assert!(output.contains("proxy fixture ready"));
}
