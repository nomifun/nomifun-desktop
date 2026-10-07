//! Execute the production cd builder in a local POSIX shell and a tempfile.
use super::change_directory_command;
use std::{path::Path, process::Command};

fn enter(cwd: &str) -> Option<(i32, String)> {
    #[cfg(windows)]
    let shell = Path::new("C:/Program Files/Git/usr/bin/sh.exe");
    #[cfg(not(windows))]
    let shell = Path::new("/bin/sh");
    if !shell.is_file() {
        eprintln!("SKIP directory command execution: POSIX sh unavailable");
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    for path in ["target", "-target", "-", "quoted ' dir", "redirect/target"] {
        std::fs::create_dir_all(dir.path().join(path)).unwrap();
        std::fs::write(dir.path().join(path).join("marker"), path).unwrap();
    }
    let script = format!(
        "{}; _nomi_cd_status=$?; if [ \"$_nomi_cd_status\" = 0 ]; then cat marker; fi; exit \"$_nomi_cd_status\"",
        change_directory_command(cwd)
    );
    let mut command = Command::new(shell);
    command
        .current_dir(dir.path())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env("PATH", "/usr/bin:/bin")
        .env("CDPATH", "redirect")
        .env("OLDPWD", "redirect")
        .args(["-c", &script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().unwrap();
    Some((
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
    ))
}

#[test]
fn relative_directory_does_not_follow_inherited_cdpath() {
    if let Some(result) = enter("target") {
        assert_eq!(result, (0, "target".into()));
    }
}

#[test]
fn leading_dash_is_a_directory_not_a_cd_option() {
    if let Some(result) = enter("-target") {
        assert_eq!(result, (0, "-target".into()));
    }
}

#[test]
fn single_dash_does_not_expand_to_oldpwd() {
    if let Some(result) = enter("-") {
        assert_eq!(result, (0, "-".into()));
    }
}

#[test]
fn quoted_directory_and_missing_directory_keep_their_meaning() {
    if let Some(result) = enter("quoted ' dir") {
        assert_eq!(result, (0, "quoted ' dir".into()));
    }
    if let Some(result) = enter("missing") {
        assert_ne!(result.0, 0);
    }
}
