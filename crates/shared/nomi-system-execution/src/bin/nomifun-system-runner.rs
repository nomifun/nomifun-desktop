fn main() {
    #[cfg(target_os = "linux")]
    {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let code = if args == [std::ffi::OsStr::new("--readonly-acquire")] {
            nomi_system_execution::nomifun_system_readonly_worker()
        } else if args == [std::ffi::OsStr::new("--stdio")] {
            match nomi_system_execution::run_stdio_helper() {
                Ok(()) => 0,
                Err(error) => { eprintln!("Linux system runner: {error}"); 74 }
            }
        } else {
            eprintln!("Usage: nomifun-system-runner --stdio");
            64
        };
        std::process::exit(code);
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("This native runner is built for Linux");
        std::process::exit(69);
    }
}
