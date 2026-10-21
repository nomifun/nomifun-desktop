// NomiFun extension under the vendored MIT license; see ../../../LICENSE.md.
// Upstream Cua attribution is retained in the existing source and license.
// NomiFun's private worker must not own the applications it opens for the user.
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        process::CommandExt,
    },
    path::PathBuf,
    process::{Command, Stdio},
};

const CONTEXT_ENV: &str = "CUA_DRIVER_RS_USER_LAUNCH_CONTEXT";
const USER_ENV: &[&str] = &[
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_DATA_DIRS",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UserLaunchContext {
    cwd: Vec<u8>,
    environment: BTreeMap<String, Option<Vec<u8>>>,
}

impl UserLaunchContext {
    pub(crate) fn read() -> io::Result<Option<Self>> {
        let Some(encoded) = std::env::var_os(CONTEXT_ENV) else {
            return Ok(None);
        };
        let context: Self = serde_json::from_slice(encoded.as_bytes()).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid user launch context: {error}"),
            )
        })?;
        if context.cwd.contains(&0)
            || !PathBuf::from(OsString::from_vec(context.cwd.clone())).is_absolute()
            || context.environment.iter().any(|(key, value)| {
                !USER_ENV.contains(&key.as_str())
                    || value.as_ref().is_some_and(|bytes| bytes.contains(&0))
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid user launch paths",
            ));
        }
        Ok(Some(context))
    }

    pub(crate) fn value(&self, key: &str) -> Option<OsString> {
        self.environment
            .get(key)
            .and_then(|value| value.as_ref())
            .map(|bytes| OsString::from_vec(bytes.clone()))
    }
}

/// One launch boundary for direct executables and xdg-open. The worker remains
/// supervised; the user application gets its own process group and no protocol
/// handles or private driver directory overrides.
pub(crate) fn command(program: impl AsRef<OsStr>) -> io::Result<Command> {
    let context = UserLaunchContext::read()?;
    let mut command = Command::new(program);
    command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env("ACCESSIBILITY_ENABLED", "1")
        .env("NO_AT_BRIDGE", "0");
    if let Some(context) = context {
        command.current_dir(OsString::from_vec(context.cwd));
        for key in USER_ENV {
            match context
                .environment
                .get(*key)
                .and_then(|value| value.as_ref())
            {
                Some(value) => {
                    command.env(key, OsString::from_vec(value.clone()));
                }
                None => {
                    command.env_remove(key);
                }
            }
        }
    }
    for (key, _) in std::env::vars_os() {
        if key.as_bytes().starts_with(b"CUA_") {
            command.env_remove(key);
        }
    }
    Ok(command)
}
