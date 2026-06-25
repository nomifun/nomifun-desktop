//! Cross-platform test fixture for the `nomi-tools` PTY/process unit tests.
//!
//! The PTY tests need a handful of deterministic child behaviours (echo stdin,
//! stay alive for N ms, exit with a code, emit output after a delay). The unix
//! programs they originally used (`cat`, `sleep`, `sh -c 'exit N'`) do not exist
//! under Windows `cmd`, so the tests spawn THIS binary instead — identical
//! behaviour on Windows and unix, no external dependencies, pure `std`.
//!
//! Subcommands (`pty_test_helper <subcommand> [args...]`):
//!   - `echo-stdin`                 read stdin line-by-line, echo each line to
//!                                  stdout (flushed), exit on EOF. Replaces `cat`.
//!   - `sleep <ms>`                 sleep `ms` milliseconds, then exit 0.
//!                                  Replaces `sleep N`.
//!   - `exit <code>`                exit immediately with `code`.
//!                                  Replaces `sh -c 'exit N'`.
//!   - `emit-after <ms> <text> <keepalive_ms>`
//!                                  sleep `ms`, print `text` + newline (flushed),
//!                                  then sleep `keepalive_ms` and exit. Models a
//!                                  process that emits delayed output then lingers.
//!
//! Kept dependency-free on purpose: it is compiled as part of the crate's normal
//! build (a `[[bin]]`) so the unit tests can locate it next to the test runner.

use std::io::{BufRead, Write};
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sub = args.first().map(String::as_str).unwrap_or("");

    match sub {
        "echo-stdin" => echo_stdin(),
        "sleep" => {
            let ms = parse_u64(args.get(1), "sleep <ms>");
            std::thread::sleep(Duration::from_millis(ms));
        }
        "exit" => {
            let code = parse_i32(args.get(1), "exit <code>");
            std::process::exit(code);
        }
        "emit-after" => {
            let delay_ms = parse_u64(args.get(1), "emit-after <ms> <text> <keepalive_ms>");
            let text = args.get(2).cloned().unwrap_or_default();
            let keepalive_ms = parse_u64(args.get(3), "emit-after <ms> <text> <keepalive_ms>");
            std::thread::sleep(Duration::from_millis(delay_ms));
            let stdout = std::io::stdout();
            let mut w = stdout.lock();
            let _ = writeln!(w, "{text}");
            let _ = w.flush();
            drop(w);
            std::thread::sleep(Duration::from_millis(keepalive_ms));
        }
        other => {
            eprintln!("pty_test_helper: unknown subcommand {other:?}");
            std::process::exit(2);
        }
    }
}

/// Read stdin line-by-line and echo each line back to stdout, flushing after
/// every line so a PTY consumer sees the echo promptly. Exits on EOF. This is
/// the cross-platform stand-in for `cat`.
fn echo_stdin() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut input = stdin.lock();
    let mut out = stdout.lock();
    let mut line = String::new();
    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                // `read_line` keeps the trailing newline; write it back verbatim.
                if out.write_all(line.as_bytes()).is_err() {
                    break;
                }
                let _ = out.flush();
            }
            Err(_) => break,
        }
    }
}

fn parse_u64(arg: Option<&String>, usage: &str) -> u64 {
    match arg.and_then(|s| s.parse::<u64>().ok()) {
        Some(v) => v,
        None => {
            eprintln!("pty_test_helper: expected {usage}");
            std::process::exit(2);
        }
    }
}

fn parse_i32(arg: Option<&String>, usage: &str) -> i32 {
    match arg.and_then(|s| s.parse::<i32>().ok()) {
        Some(v) => v,
        None => {
            eprintln!("pty_test_helper: expected {usage}");
            std::process::exit(2);
        }
    }
}
