#[cfg(any(unix, windows))]
use std::{
    env,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{self, Stdio},
    time::Duration,
};

#[cfg(any(unix, windows))]
use nomi_process_runtime::ChildProcessBuilder;

#[cfg(unix)]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("child-process parent-death harness failed: {error}");
        process::exit(2);
    }
    // SAFETY: the harness intentionally bypasses Rust drops and Tokio cleanup.
    unsafe { libc::_exit(0) }
}

#[cfg(windows)]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("child-process parent-death harness failed: {error}");
        process::exit(2);
    }
    use windows_sys::Win32::System::Threading::ExitProcess;
    // SAFETY: the harness intentionally bypasses Rust drops so the process
    // owner must react to host termination.
    unsafe { ExitProcess(0) }
}

#[cfg(not(any(unix, windows)))]
fn main() {}

#[cfg(any(unix, windows))]
async fn run() -> io::Result<()> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    #[cfg(unix)]
    let expected_args = 3;
    #[cfg(windows)]
    let expected_args = 4;
    if args.len() != expected_args {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected helper, leader marker, grandchild marker, and optional Windows exit gate",
        ));
    }
    let helper = args[0].clone();
    let leader_marker = PathBuf::from(&args[1]);
    let grandchild_marker = PathBuf::from(&args[2]);
    let mut builder = ChildProcessBuilder::new(helper);
    builder
        .args([
            OsString::from("spawn-grandchild"),
            grandchild_marker.as_os_str().to_owned(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = builder.spawn()?;
    let leader = child
        .id()
        .ok_or_else(|| io::Error::other("child process exited before publishing its PID"))?;
    wait_for_pid_marker(&grandchild_marker).await?;
    write_pid_atomically(&leader_marker, leader)?;
    #[cfg(windows)]
    wait_for_gate(Path::new(&args[3])).await?;
    std::mem::forget(child);
    Ok(())
}

#[cfg(any(unix, windows))]
async fn wait_for_pid_marker(path: &Path) -> io::Result<u32> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(contents) = fs::read_to_string(path)
                && let Ok(pid) = contents.trim().parse::<u32>()
            {
                return Ok(pid);
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "grandchild PID marker timed out"))?
}

#[cfg(windows)]
async fn wait_for_gate(path: &Path) -> io::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if path.is_file() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "exit gate timed out"))
}

#[cfg(any(unix, windows))]
fn write_pid_atomically(path: &Path, pid: u32) -> io::Result<()> {
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
            Ok(mut file) => {
                let result = (|| {
                    writeln!(file, "{pid}")?;
                    file.flush()?;
                    file.sync_all()?;
                    drop(file);
                    fs::rename(&temporary_path, path)
                })();
                if result.is_err() {
                    let _ = fs::remove_file(&temporary_path);
                }
                return result;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary PID marker",
    ))
}
