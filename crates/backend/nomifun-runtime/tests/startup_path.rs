#![cfg(any(target_os = "macos", target_os = "linux"))]

#[test]
fn gui_startup_without_shell_uses_account_profile_and_bounds_inherited_stdout() {
    let directory = tempfile::tempdir().expect("private fake home");
    let fixture_bin = directory.path().join("account-profile-bin");
    std::fs::create_dir(&fixture_bin).unwrap();
    let quoted_bin = fixture_bin.to_string_lossy().replace('\'', "'\\''");
    let profile = format!("export PATH='{quoted_bin}':\"$PATH\"\n/bin/sleep 60 &\n");
    // The actual account may select zsh, bash, or sh. Every startup file is in
    // this private HOME; the operator's real shell profiles remain untouched.
    for name in [".zprofile", ".zshrc", ".bash_profile", ".bashrc", ".profile"] {
        std::fs::write(directory.path().join(name), &profile).unwrap();
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_startup_path_harness"))
        .arg(&fixture_bin)
        .env_remove("SHELL")
        .env_remove("ZDOTDIR")
        .env("HOME", directory.path())
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("standalone startup PATH harness");
    assert!(output.status.success(), "startup PATH harness failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("GUI startup without SHELL loaded account profile"));
}
