//! macOS Seatbelt write-containment sandbox for the `Bash` tool (design §3.6
//! "Bash/Edit/Write 执行沙箱(macOS Seatbelt 优先)").
//!
//! Opt-in (`tools.bash_sandbox`, default off). When enabled on macOS, shell
//! commands run under `sandbox-exec` with a profile that **denies all
//! file-writes except** to the workspace root(s), the system temp dirs, and the
//! standard write devices (`/dev/null`, stdout/stderr/tty, `/dev/fd`). Reads,
//! network and exec are left allowed.
//!
//! # Why this complements `path_guard`
//!
//! `path_guard` only constrains *our own* Write/Edit/ApplyPatch tools.
//! Seatbelt constrains **every write syscall of every subprocess** a Bash
//! command spawns (a `make install` into `/usr/local`, a script touching
//! `~/.bashrc`), enforced by the kernel.
//!
//! # Honest scope
//!
//! This protects the broader filesystem from *writes* outside the workspace +
//! temp. It is NOT a full adversarial sandbox: network and process execution
//! remain allowed, so it guards against accidental/buggy damage, not a
//! determined adversary. Process hardening (ptrace/exec restrictions) is a
//! separate concern.
//!
//! # Two gotchas this module gets right (both caught by running it on macOS)
//!
//! 1. **Canonical paths**: `/tmp` is a symlink to `/private/tmp`; Seatbelt
//!    `subpath` matches the kernel's canonical path, so roots are canonicalised.
//! 2. **Device writes**: a bare `(deny file-write*)` blocks `> /dev/null`
//!    redirects, breaking ordinary commands — the standard devices are
//!    explicitly re-allowed.

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};

/// Whether the sandbox can be used (macOS + `sandbox-exec` present).
pub fn is_supported() -> bool {
    Path::new("/usr/bin/sandbox-exec").exists()
}

/// Escape a path for embedding inside a Seatbelt profile string literal.
fn escape(path: &str) -> String {
    path.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Canonicalise a path for Seatbelt (resolve `/tmp`→`/private/tmp`, symlinks,
/// `..`). Falls back to the original on failure (e.g. not-yet-existing).
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Build a Seatbelt profile that allows everything by default, denies all
/// file-writes, then re-allows writes under each canonicalised root plus the
/// system temp dirs and the standard write devices.
pub fn write_sandbox_profile(write_roots: &[PathBuf]) -> String {
    let mut allowed: Vec<PathBuf> = Vec::new();
    for r in write_roots {
        allowed.push(canonical(r));
    }
    // System temp dirs (canonicalised) — builds/tools need a scratch space.
    // Kept tight: the per-user TMPDIR and /tmp, NOT the broad /var/folders tree.
    if let Ok(tmp) = std::env::var("TMPDIR") {
        allowed.push(canonical(Path::new(&tmp)));
    }
    allowed.push(canonical(Path::new("/private/tmp")));

    let mut profile = String::from("(version 1)\n(allow default)\n(deny file-write*)\n");

    if !allowed.is_empty() {
        profile.push_str("(allow file-write*\n");
        for p in &allowed {
            profile.push_str(&format!("  (subpath \"{}\")\n", escape(&p.to_string_lossy())));
        }
        profile.push_str(")\n");
    }

    // Standard write devices, or ordinary `>/dev/null` redirects break.
    profile.push_str(
        "(allow file-write*\n  \
         (literal \"/dev/null\")\n  \
         (literal \"/dev/stdout\")\n  \
         (literal \"/dev/stderr\")\n  \
         (literal \"/dev/tty\")\n  \
         (literal \"/dev/dtracehelper\")\n  \
         (subpath \"/dev/fd\")\n)\n",
    );
    profile
}

/// Build the argv that runs `inner_argv` under the sandbox: `sandbox-exec -p
/// <profile> <inner_argv...>`.
pub fn wrap_command(profile: &str, inner_argv: &[&str]) -> Vec<String> {
    let mut argv = vec!["/usr/bin/sandbox-exec".to_string(), "-p".to_string(), profile.to_string()];
    argv.extend(inner_argv.iter().map(|s| s.to_string()));
    argv
}

/// Dynamic-linker injection env vars that let an inherited environment load
/// arbitrary code into a child — stripped from sandboxed subprocesses so a
/// command cannot be subverted via the agent's inherited env (§3.6 进程加固).
pub const DANGEROUS_ENV_VARS: &[&str] = &[
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FRAMEWORK_PATH",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
];

/// Remove the dynamic-linker injection vars from a command's environment.
pub fn harden_env(cmd: &mut tokio::process::Command) {
    for var in DANGEROUS_ENV_VARS {
        cmd.env_remove(var);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn profile_lists_canonical_roots_and_devices() {
        let dir = tempfile::tempdir().unwrap();
        let profile = write_sandbox_profile(&[dir.path().to_path_buf()]);
        assert!(profile.contains("(deny file-write*)"));
        // The root appears as a canonical subpath.
        let canon = dir.path().canonicalize().unwrap();
        assert!(
            profile.contains(&format!("(subpath \"{}\")", canon.to_string_lossy())),
            "profile must allow the canonical root, got:\n{profile}"
        );
        assert!(profile.contains("/dev/null"), "must re-allow /dev/null");
    }

    // Enforcement is verifiable on this macOS host via the real sandbox-exec.
    #[test]
    fn sandbox_blocks_out_of_root_writes_but_allows_in_root() {
        if !is_supported() {
            return; // sandbox-exec unavailable — skip
        }
        let root = tempfile::tempdir().unwrap();
        let canon_root = root.path().canonicalize().unwrap();
        let profile = write_sandbox_profile(&[canon_root.clone()]);

        // In-root write succeeds.
        let inside = canon_root.join("ok.txt");
        let argv = wrap_command(
            &profile,
            &["/bin/sh", "-c", &format!("echo hi > {}", inside.display())],
        );
        let status = Command::new(&argv[0]).args(&argv[1..]).status().unwrap();
        assert!(status.success(), "in-root write should succeed");
        assert_eq!(std::fs::read_to_string(&inside).unwrap().trim(), "hi");

        // Out-of-root write is blocked by the kernel. Target $HOME (not temp,
        // which the profile intentionally allows). Cleaned up either way.
        let home = std::env::var("HOME").expect("HOME set");
        let outside = Path::new(&home).join(".nomi_sandbox_escape_test.txt");
        let _ = std::fs::remove_file(&outside);
        let argv = wrap_command(
            &profile,
            &["/bin/sh", "-c", &format!("echo hi > {}", outside.display())],
        );
        let _ = Command::new(&argv[0]).args(&argv[1..]).status().unwrap();
        let escaped = outside.exists();
        let _ = std::fs::remove_file(&outside);
        assert!(!escaped, "out-of-root write (to $HOME) must be blocked by the sandbox");

        // A normal command that redirects to /dev/null still works.
        let argv = wrap_command(&profile, &["/bin/sh", "-c", "echo hi > /dev/null && echo OK"]);
        let out = Command::new(&argv[0]).args(&argv[1..]).output().unwrap();
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("OK"),
            "redirect to /dev/null must work under the sandbox"
        );
    }

    #[tokio::test]
    async fn harden_env_strips_injection_vars() {
        // Set an injection var ON THE COMMAND, harden it, and confirm the child
        // does not see it (no global env mutation needed).
        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.env("DYLD_INSERT_LIBRARIES", "/tmp/evil.dylib")
            .arg("-c")
            .arg("printf '%s' \"$DYLD_INSERT_LIBRARIES\"");
        harden_env(&mut cmd);
        let out = cmd.output().await.unwrap();
        assert!(
            String::from_utf8_lossy(&out.stdout).is_empty(),
            "DYLD_INSERT_LIBRARIES must be stripped from the child env"
        );
    }
}
