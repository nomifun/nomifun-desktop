//! Windows script entry points for the shared process supervisor.
//!
//! Batch files have cmd's command-string contract, not a native argv array.
//! Use the standard library's batch quoting rules, reject line breaks before
//! spawning, and leave explicit `cmd /c` source untouched. PowerShell scripts
//! receive a splatted array of data literals through an encoded command.
use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::OsStrExt,
    path::{Component, Path, Prefix},
    ptr,
};

use base64::{Engine, engine::general_purpose::STANDARD};

use super::{
    ProcessError, append_quoted, finish_command_line, invalid_command, powershell_payload,
};

pub(super) enum ScriptKind {
    Batch,
    PowerShell,
}

pub(super) fn validate_batch_cwd(cwd: &Path) -> Result<(), ProcessError> {
    if !matches!(cwd.components().next(),Some(Component::Prefix(prefix)) if matches!(prefix.kind(),Prefix::UNC(..)|Prefix::VerbatimUNC(..)))
    {
        return Ok(());
    }
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
    };
    let key = "Software\\Microsoft\\Command Processor\0"
        .encode_utf16()
        .collect::<Vec<_>>();
    let name = "DisableUNCCheck\0".encode_utf16().collect::<Vec<_>>();
    let mut setting = 0u32;
    let mut length = std::mem::size_of::<u32>() as u32;
    // Read cmd.exe's existing per-user setting; never write configuration or
    // silently map a network drive just to manufacture a different cwd.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            (&mut setting as *mut u32).cast(),
            &mut length,
        )
    };
    let allowed = match result {
        ERROR_SUCCESS => setting != 0,
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => false,
        error => {
            return Err(ProcessError::InvalidWorkingDirectory {
                path: cwd.to_owned(),
                reason: format!(
                    "Cannot verify cmd.exe's existing DisableUNCCheck setting: {}",
                    std::io::Error::from_raw_os_error(error as i32)
                ),
            });
        }
    };
    if !allowed {
        return Err(ProcessError::InvalidWorkingDirectory {path:cwd.to_owned(),reason:"cmd.exe would replace this UNC working directory with the Windows directory because its existing DisableUNCCheck setting is not enabled; the requested batch script was not started".into()});
    }
    Ok(())
}

pub(super) fn script_kind(program: &OsStr) -> Option<ScriptKind> {
    // Match Win32 aliases too: a trailing dot/space does not turn a batch file
    // into a native executable. Final path resolution still happens at spawn.
    let filename = Path::new(program)
        .file_name()?
        .to_str()?
        .trim_end_matches(['.', ' ']);
    let extension = filename.rsplit_once('.')?.1;
    if extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat") {
        Some(ScriptKind::Batch)
    } else if extension.eq_ignore_ascii_case("ps1") {
        Some(ScriptKind::PowerShell)
    } else {
        None
    }
}

pub(super) fn batch_command_line(
    cmd: &OsStr,
    script: &OsStr,
    args: &[OsString],
) -> Result<Vec<u16>, ProcessError> {
    let mut line = Vec::new();
    append_quoted(cmd, &mut line)?;
    line.extend(" /d /e:ON /v:OFF /s /c \"".encode_utf16());
    // A filename cannot contain quotes or end with a separator. Unlike argv,
    // a quoted script name needs no CRT escaping of directory separators.
    let path = script.encode_wide().collect::<Vec<_>>();
    if path.iter().any(|unit| matches!(*unit, 0 | 10 | 13 | 34)) || path.last() == Some(&92) {
        return Err(invalid_command(
            "The Windows batch script path cannot be represented by cmd.exe without changing it",
        ));
    }
    line.push(34);
    for unit in path {
        append_batch_unit(unit, &mut line);
    }
    line.push(34);
    for argument in args {
        let units = argument.encode_wide().collect::<Vec<_>>();
        if units.iter().any(|unit| matches!(*unit, 0 | 10 | 13)) {
            return Err(invalid_command(
                "Windows batch script arguments cannot contain NUL, CR or LF; cmd.exe cannot preserve these literal arguments",
            ));
        }
        line.extend([32, 34]);
        let mut backslashes = 0;
        for unit in units {
            if unit == 92 {
                backslashes += 1;
            } else {
                if unit == 34 {
                    line.extend(std::iter::repeat_n(92, backslashes));
                    line.push(34);
                }
                backslashes = 0;
            }
            append_batch_unit(unit, &mut line);
        }
        line.extend(std::iter::repeat_n(92, backslashes));
        line.push(34);
    }
    line.push(34);
    // cmd.exe's documented limit is shorter than CreateProcessW's limit.
    if line.len() > 8191 {
        return Err(invalid_command(
            "Windows batch command line exceeds cmd.exe's 8,191 UTF-16 code-unit limit",
        ));
    }
    finish_command_line(line)
}

fn append_batch_unit(unit: u16, line: &mut Vec<u16>) {
    if unit == 37 {
        // The same percent encoding used by Rust's std::process::Command:
        // with command extensions enabled `%cd:~,%` expands to empty and
        // prevents a user `%NAME%` token becoming environment substitution.
        // https://github.com/rust-lang/rust/blob/master/library/std/src/sys/args/windows.rs
        line.extend("%%cd:~,".encode_utf16());
    }
    line.push(unit);
}

pub(super) fn powershell_arguments(
    program: &OsStr,
    args: &[OsString],
) -> Result<Vec<OsString>, ProcessError> {
    let literal = |value: &OsStr| -> Result<String, ProcessError> {
        let value = value
            .to_str()
            .ok_or_else(|| invalid_command("PowerShell script arguments must be valid Unicode"))?;
        if value.contains('\0') {
            return Err(invalid_command(
                "PowerShell script arguments cannot contain NUL",
            ));
        }
        Ok(format!("'{}'", value.replace('\'', "''")))
    };
    let arguments = args
        .iter()
        .map(|arg| literal(arg))
        .collect::<Result<Vec<_>, _>>()?;
    let source = format!(
        "$nomifunScriptArguments = @({})\n& {} @nomifunScriptArguments",
        arguments.join(","),
        literal(program)?
    );
    let bytes = powershell_payload(&source)
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    Ok([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-OutputFormat",
        "Text",
        "-EncodedCommand",
    ]
    .into_iter()
    .map(OsString::from)
    .chain([OsString::from(STANDARD.encode(bytes))])
    .collect())
}
