use std::{
    env,
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{self, Command},
    thread,
    time::Duration,
};

const LONG_SLEEP: Duration = Duration::from_secs(60);
const UTF8_SAMPLE: &str = "中文🙂";

fn main() {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        fail("missing helper subcommand");
    };

    match command {
        "exit" => {
            require_len(&args, 2);
            process::exit(parse_i32(&args[1], "exit code"));
        }
        "sleep" => {
            require_len(&args, 2);
            thread::sleep(Duration::from_millis(parse_u64(&args[1], "sleep duration")));
        }
        "echo-stdin" => {
            require_len(&args, 1);
            copy_stdin().unwrap_or_else(|error| fail_io("echo stdin", error));
        }
        "emit-interleaved" => {
            require_len(&args, 1);
            emit_interleaved().unwrap_or_else(|error| fail_io("emit interleaved", error));
        }
        "emit-split-utf8" => {
            require_len(&args, 1);
            emit_split_utf8().unwrap_or_else(|error| fail_io("emit split UTF-8", error));
        }
        "flood" => {
            require_len(&args, 2);
            flood(parse_u64(&args[1], "flood byte count"))
                .unwrap_or_else(|error| fail_io("flood stdout", error));
        }
        "spawn-grandchild" => {
            require_len(&args, 2);
            spawn_grandchild(Path::new(&args[1]))
                .unwrap_or_else(|error| fail_io("spawn grandchild", error));
        }
        "ignore-interrupt" => {
            require_len(&args, 1);
            ignore_interrupt().unwrap_or_else(|error| fail_io("ignore interrupt", error));
            emit_ready().unwrap_or_else(|error| fail_io("emit interrupt readiness", error));
            thread::sleep(LONG_SLEEP);
        }
        "write-pid" => {
            require_len(&args, 2);
            write_pid_atomically(Path::new(&args[1]), process::id())
                .unwrap_or_else(|error| fail_io("write PID marker", error));
        }
        _ => fail("unknown helper subcommand"),
    }
}

fn copy_stdin() -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    io::copy(&mut input, &mut output)?;
    output.flush()
}

fn emit_interleaved() -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    stdout.write_all(b"stdout-1\n")?;
    stdout.flush()?;
    stderr.write_all(b"stderr-1\n")?;
    stderr.flush()?;
    stdout.write_all(b"stdout-2\n")?;
    stdout.flush()?;
    stderr.write_all(b"stderr-2\n")?;
    stderr.flush()
}

fn emit_split_utf8() -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    for byte in UTF8_SAMPLE.as_bytes() {
        stdout.write_all(&[*byte])?;
        stdout.flush()?;
    }
    Ok(())
}

fn emit_ready() -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(b"ready\n")?;
    stdout.flush()
}

fn flood(mut remaining: u64) -> io::Result<()> {
    const BLOCK: [u8; 8 * 1024] = [b'x'; 8 * 1024];
    let mut stdout = io::stdout().lock();
    while remaining > 0 {
        let count = remaining.min(BLOCK.len() as u64) as usize;
        stdout.write_all(&BLOCK[..count])?;
        remaining -= count as u64;
    }
    stdout.flush()
}

fn spawn_grandchild(marker: &Path) -> io::Result<()> {
    let executable = env::current_exe()?;
    let mut grandchild = Command::new(executable).args(["sleep", "60000"]).spawn()?;
    if let Err(error) = write_pid_atomically(marker, grandchild.id()) {
        let _ = grandchild.kill();
        let _ = grandchild.wait();
        return Err(error);
    }
    thread::sleep(LONG_SLEEP);
    let _ = grandchild.wait();
    Ok(())
}

fn write_pid_atomically(path: &Path, pid: u32) -> io::Result<()> {
    let (temporary_path, mut temporary) = create_temporary_marker(path, pid)?;
    let result = (|| {
        writeln!(temporary, "{pid}")?;
        temporary.flush()?;
        temporary.sync_all()?;
        drop(temporary);
        fs::rename(&temporary_path, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn create_temporary_marker(path: &Path, pid: u32) -> io::Result<(PathBuf, fs::File)> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "marker has no file name"))?;
    for attempt in 0..100_u32 {
        let mut temporary_name = OsString::from(".");
        temporary_name.push(file_name);
        temporary_name.push(format!(".{pid}.{attempt}.tmp"));
        let temporary_path = parent.join(temporary_name);
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary PID marker",
    ))
}

#[cfg(windows)]
fn ignore_interrupt() -> io::Result<()> {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

    unsafe extern "system" fn ignore_control_event(_control_type: u32) -> i32 {
        1
    }

    // SAFETY: the handler has the required system ABI, remains valid for the
    // process lifetime, and reports both CTRL+C and CTRL+BREAK as handled.
    if unsafe { SetConsoleCtrlHandler(Some(ignore_control_event), 1) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn ignore_interrupt() -> io::Result<()> {
    // SAFETY: installing SIG_IGN for SIGINT changes only this helper's signal disposition.
    if unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) } == libc::SIG_ERR {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn require_len(args: &[OsString], expected: usize) {
    if args.len() != expected {
        fail("invalid helper arguments");
    }
}

fn parse_i32(value: &OsStr, label: &str) -> i32 {
    value
        .to_str()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| fail(&format!("invalid {label}")))
}

fn parse_u64(value: &OsStr, label: &str) -> u64 {
    value
        .to_str()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| fail(&format!("invalid {label}")))
}

fn fail_io(action: &str, error: io::Error) -> ! {
    fail(&format!("{action}: {error}"))
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(2);
}
