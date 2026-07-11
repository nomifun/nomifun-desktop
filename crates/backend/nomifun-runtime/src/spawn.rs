//! Source-compatible legacy command facade backed by `nomi-execution`.
//!
//! New Agent command execution uses `ProcessSupervisor` directly. This facade
//! keeps non-Agent backend callers stable while ensuring process environment
//! and platform ownership primitives have a single implementation.

use std::{
    ffi::{OsStr, OsString},
    io,
    path::Path,
    process::Stdio,
};

#[cfg(unix)]
use std::os::fd::{OwnedFd, RawFd};
use tokio::process::Child;

use crate::resolver::resolve_command_path;

pub struct Builder {
    inner: nomi_execution::CommandBuilder,
    mode: Mode,
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    Default,
    CleanCli,
}

impl Builder {
    pub fn new<S: AsRef<OsStr>>(program: S) -> Self {
        Self {
            inner: nomi_execution::CommandBuilder::new(resolve_program(program.as_ref())),
            mode: Mode::Default,
        }
    }

    pub fn clean_cli<S: AsRef<OsStr>>(program: S) -> Self {
        Self {
            inner: nomi_execution::CommandBuilder::clean_cli(resolve_program(program.as_ref())),
            mode: Mode::CleanCli,
        }
    }

    pub fn hand_off(&mut self) -> &mut Self {
        self.inner.hand_off();
        self
    }

    #[cfg(unix)]
    pub fn inherit_fds(&mut self, mappings: Vec<(RawFd, OwnedFd)>) -> &mut Self {
        self.inner.inherit_fds(mappings);
        self
    }

    pub fn arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        self.inner.arg(arg);
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.inner.args(args);
        self
    }

    pub fn env<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.inner.env(key, value);
        self
    }

    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.inner.envs(vars);
        self
    }

    pub fn env_remove<K: AsRef<OsStr>>(&mut self, key: K) -> &mut Self {
        self.inner.env_remove(key);
        self
    }

    pub fn current_dir<P: AsRef<Path>>(&mut self, dir: P) -> &mut Self {
        self.inner.current_dir(dir);
        self
    }

    pub fn stdin<T: Into<Stdio>>(&mut self, value: T) -> &mut Self {
        self.inner.stdin(value);
        self
    }

    pub fn stdout<T: Into<Stdio>>(&mut self, value: T) -> &mut Self {
        self.inner.stdout(value);
        self
    }

    pub fn stderr<T: Into<Stdio>>(&mut self, value: T) -> &mut Self {
        self.inner.stderr(value);
        self
    }

    pub fn spawn(self) -> io::Result<Child> {
        self.inner.spawn()
    }

    pub async fn output(self) -> io::Result<std::process::Output> {
        self.inner.output().await
    }
}

impl std::fmt::Debug for Builder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Builder")
            .field("mode", &self.mode)
            .field("command", self.inner.as_std())
            .finish()
    }
}

impl std::fmt::Display for Builder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.inner, formatter)
    }
}

pub async fn kill_process_tree(child: &mut Child) -> io::Result<()> {
    nomi_execution::kill_legacy_process_tree(child).await
}

fn resolve_program(program: &OsStr) -> OsString {
    if let Some(program) = program.to_str()
        && !program.is_empty()
        && !program.contains('/')
        && !program.contains('\\')
        && let Some(path) = resolve_command_path(program)
    {
        return path.into_os_string();
    }
    program.to_os_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_command(script: &str) -> Builder {
        #[cfg(windows)]
        {
            let mut builder = Builder::new("powershell");
            builder.args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ]);
            builder
        }
        #[cfg(unix)]
        {
            let mut builder = Builder::new("sh");
            builder.args(["-c", script]);
            builder
        }
    }

    #[tokio::test]
    async fn clean_cli_captures_stdout() {
        let mut builder = if cfg!(windows) {
            Builder::clean_cli("powershell")
        } else {
            Builder::clean_cli("sh")
        };
        #[cfg(windows)]
        builder.args(["-NoProfile", "-Command", "Write-Output hello"]);
        #[cfg(unix)]
        builder.args(["-c", "printf hello"]);

        let output = builder.output().await.expect("clean command should run");

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }

    #[tokio::test]
    async fn builder_allows_stdio_override() {
        let mut builder = shell_command(if cfg!(windows) {
            "Write-Output hello"
        } else {
            "printf hello"
        });
        builder.stdout(Stdio::piped());

        let output = builder.output().await.expect("command should run");

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }

    #[test]
    fn display_preserves_command_preview() {
        let mut builder = Builder::new("/usr/local/bin/bun");
        builder
            .current_dir("/tmp/work dir")
            .env("FOO", "bar baz")
            .args(["x", "--flag", "with space"]);

        let preview = format!("{builder}");

        assert!(preview.contains("bun"));
        assert!(preview.contains("--flag"));
        assert!(preview.contains("with space"));
    }

    #[test]
    fn bare_bun_resolution_remains_in_the_runtime_facade() {
        let program = resolve_program(OsStr::new("bun"));
        if let Some(resolved) = resolve_command_path("bun") {
            assert_eq!(program, resolved.into_os_string());
        }
    }
}
