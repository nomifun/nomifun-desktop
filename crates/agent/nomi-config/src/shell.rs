use tokio::process::Command;

const POWERSHELL_EXE: &str = "powershell.exe";
const POWERSHELL_ARGS: &[&str] = &[
    "-NoLogo",
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-Command",
];
const SH_ARGS: &[&str] = &["-c"];

pub struct ShellInfo {
    pub program: &'static str,
    pub args_before_command: &'static [&'static str],
    pub syntax_name: &'static str,
}

pub fn shell_info() -> ShellInfo {
    if cfg!(windows) {
        ShellInfo {
            program: POWERSHELL_EXE,
            args_before_command: POWERSHELL_ARGS,
            syntax_name: "PowerShell",
        }
    } else {
        ShellInfo {
            program: "sh",
            args_before_command: SH_ARGS,
            syntax_name: "POSIX sh",
        }
    }
}

pub fn shell_command_args(command_str: &str) -> Vec<String> {
    let info = shell_info();
    let mut args = info
        .args_before_command
        .iter()
        .map(|arg| (*arg).to_owned())
        .collect::<Vec<_>>();
    args.push(shell_command_payload(command_str));
    args
}

pub fn shell_command_builder(command_str: &str) -> Command {
    let info = shell_info();
    let mut cmd = Command::new(info.program);
    cmd.args(shell_command_args(command_str));
    // CREATE_NO_WINDOW: don't flash a console window when the host is a GUI app.
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    cmd
}

fn shell_command_payload(command_str: &str) -> String {
    if cfg!(windows) {
        powershell_payload(command_str)
    } else {
        command_str.to_owned()
    }
}

#[cfg(windows)]
fn powershell_payload(command_str: &str) -> String {
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         $global:LASTEXITCODE = $null\n\
         try {{\n\
         & {{\n\
         {command_str}\n\
         }}\n\
         if ($null -ne $global:LASTEXITCODE) {{ exit $global:LASTEXITCODE }}\n\
         if (-not $?) {{ exit 1 }}\n\
         exit 0\n\
         }} catch {{\n\
         [Console]::Error.WriteLine($_.Exception.Message)\n\
         exit 1\n\
         }}"
    )
}

#[cfg(not(windows))]
fn powershell_payload(command_str: &str) -> String {
    command_str.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_info_returns_platform_appropriate_values() {
        let info = shell_info();
        if cfg!(windows) {
            assert_eq!(info.program, "powershell.exe");
            assert_eq!(info.args_before_command, POWERSHELL_ARGS);
            assert_eq!(info.syntax_name, "PowerShell");
        } else {
            assert_eq!(info.program, "sh");
            assert_eq!(info.args_before_command, SH_ARGS);
            assert_eq!(info.syntax_name, "POSIX sh");
        }
    }

    #[tokio::test]
    async fn shell_command_builder_allows_env_and_cwd() {
        let tmp = std::env::temp_dir();
        let cmd_str = if cfg!(windows) {
            "Write-Output $env:MY_VAR"
        } else {
            "echo $MY_VAR"
        };
        let output = shell_command_builder(cmd_str)
            .env("MY_VAR", "test_value")
            .current_dir(&tmp)
            .output()
            .await
            .expect("builder failed");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("test_value"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn shell_command_builder_accepts_powershell_syntax_on_windows() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("proof.txt"), "ok").unwrap();
        let output = shell_command_builder(
            "if (Test-Path proof.txt) { Get-Content proof.txt } else { exit 9 }",
        )
        .current_dir(tmp.path())
        .output()
        .await
        .expect("builder failed");

        assert!(
            output.status.success(),
            "status: {:?}",
            output.status.code()
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("ok"), "stdout: {stdout}");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn shell_command_builder_preserves_native_exit_code_on_windows() {
        let output = shell_command_builder("cmd /c exit 7")
            .output()
            .await
            .expect("builder failed");

        assert_eq!(output.status.code(), Some(7));
    }
}
