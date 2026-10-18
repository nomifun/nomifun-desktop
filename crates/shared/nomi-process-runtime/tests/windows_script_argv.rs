#![cfg(windows)]

use nomi_process_runtime::{
    ChildProcessBuilder, CommandSpec, NormalizedProcessRequest, OutputCursor, OutputStream,
    PollResult, ProcessError, ProcessOutcome, ProcessOwner, ProcessPolicy, ProcessSupervisor,
    SupervisorConfig, Transport,
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::Path,
    time::{Duration, Instant},
};

fn request(
    script: &Path,
    args: &[String],
    cwd: &Path,
    transport: Transport,
) -> NormalizedProcessRequest {
    NormalizedProcessRequest {
        owner: ProcessOwner::new(uuid::Uuid::now_v7(), uuid::Uuid::now_v7()),
        command: CommandSpec::Program {
            program: script.as_os_str().to_owned(),
            args: args.iter().map(OsString::from).collect(),
        },
        cwd: cwd.to_owned(),
        env: BTreeMap::new(),
        transport,
        policy: ProcessPolicy::default(),
    }
}

async fn run(mut process: NormalizedProcessRequest) -> String {
    process
        .env
        .insert("NOMIFUN_ARGV_TEST_VALUE".into(), "SHOULD_NOT_EXPAND".into());
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor.start(process).await.expect("script starts");
    let result = supervisor
        .poll(
            &handle.owner,
            &handle.session_id,
            OutputCursor::START,
            Instant::now() + Duration::from_secs(20),
        )
        .await
        .expect("poll script");
    let PollResult::Finished(ProcessOutcome::Exited {
        code,
        output,
        cleanup,
        ..
    }) = result
    else {
        panic!("script did not exit: {result:?}");
    };
    assert_eq!(code, Some(0), "script output: {}", output.text());
    assert!(
        cleanup.reaped && cleanup.errors.is_empty(),
        "script cleanup: {cleanup:?}"
    );
    assert!(supervisor.shutdown().await.is_exact());
    // PowerShell module-loading progress is a real stderr stream; it is not
    // part of the script's independently reported stdout protocol.
    output
        .chunks
        .iter()
        .filter(|chunk| chunk.stream != OutputStream::Stderr)
        .map(|chunk| chunk.text.as_str())
        .collect::<String>()
        .replace("\r\n", "\n")
}

#[tokio::test]
async fn batch_shims_roundtrip_native_argv_under_pipe_and_conpty() {
    let root = tempfile::tempdir().unwrap();
    let directory = root
        .path()
        .join("script path & %NOMIFUN_ARGV_TEST_VALUE% 'quoted'");
    std::fs::create_dir(&directory).unwrap();
    let helper = directory.join("argv-helper.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_process_test_helper"), &helper).unwrap();
    let arguments = [
        "",
        "two words",
        " leading and trailing ",
        "\"",
        "quote \" and trailing \\",
        "a&b|c<d>e^f(g)!h!",
        "%NOMIFUN_ARGV_TEST_VALUE%",
        "$(New-Item PWNED) `tick` 'literal'",
        "中文🙂",
        "tab\tvalue",
        "C:\\trailing\\",
        "\"&echo PWNED>injected.txt&\"",
    ]
    .map(String::from);
    let expected = arguments
        .iter()
        .map(|arg| format!("{:?}\n", OsString::from(arg)))
        .collect::<String>();
    // The script does exactly the forwarding npm shims do. The native helper
    // independently reports the actual argv delivered after cmd parsing.
    for extension in ["cmd", "bat"] {
        let script = directory.join(format!(" leading fixture.{extension}"));
        // %~dp0 is expanded by the fixture itself, after the launch encoding.
        std::fs::write(
            &script,
            "@echo off\r\n\"%~dp0argv-helper.exe\" record-argv \"%NOMIFUN_ARGV_REPORT%\" %*\r\n",
        )
        .unwrap();
        let report = directory.join("argv-report.txt");
        let mut builder = ChildProcessBuilder::clean_cli(&script);
        builder
            .args(&arguments)
            .current_dir(&directory)
            .env("NOMIFUN_ARGV_TEST_VALUE", "SHOULD_NOT_EXPAND")
            .env("NOMIFUN_ARGV_REPORT", &report);
        let child = builder.output().await.expect("raw stdio owner starts shim");
        assert!(
            child.status.success(),
            "raw stdio stderr: {}",
            String::from_utf8_lossy(&child.stderr)
        );
        assert_eq!(
            String::from_utf8(child.stdout)
                .unwrap()
                .replace("\r\n", "\n"),
            expected
        );
        assert_eq!(std::fs::read_to_string(&report).unwrap(), expected);
        std::fs::remove_file(&report).unwrap();
        for transport in [
            Transport::Pipe,
            Transport::Pty {
                cols: 120,
                rows: 30,
            },
        ] {
            let mut process = request(&script, &arguments, &directory, transport);
            process
                .env
                .insert("NOMIFUN_ARGV_REPORT".into(), report.as_os_str().to_owned());
            let output = run(process).await;
            // ConPTY can emit terminal control sequences around real output.
            for line in expected.lines() {
                assert!(output.contains(line), "expected {line:?}, got {output:?}");
            }
            if transport == Transport::Pipe {
                assert_eq!(output, expected);
            }
            assert_eq!(
                std::fs::read_to_string(&report).unwrap(),
                expected,
                "independent native argv report for {transport:?}"
            );
            std::fs::remove_file(&report).unwrap();
            assert!(!directory.join("injected.txt").exists());
        }
    }
}

#[tokio::test]
async fn batch_unrepresentable_line_breaks_fail_before_effect() {
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("fixture.cmd");
    std::fs::write(&script, "@echo off\r\necho BAD>started.txt\r\n").unwrap();
    for value in ["line\nbreak", "line\rbreak"] {
        let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
        let result = supervisor
            .start(request(
                &script,
                &[value.into()],
                root.path(),
                Transport::Pipe,
            ))
            .await;
        assert!(
            matches!(result, Err(ProcessError::InvalidCommand { ref reason }) if reason.contains("CR or LF")),
            "{result:?}"
        );
        assert!(!root.path().join("started.txt").exists());
        assert!(supervisor.shutdown().await.is_exact());
        let mut builder = ChildProcessBuilder::clean_cli(&script);
        builder.arg(value).current_dir(root.path());
        let error = builder
            .output()
            .await
            .expect_err("raw stdio owner rejects line break before spawn");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!root.path().join("started.txt").exists());
    }
}

#[tokio::test]
async fn powershell_script_preserves_empty_quotes_unicode_newlines_and_metacharacters() {
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join(" leading script 'quoted' & %literal%.ps1");
    std::fs::write(
        &script,
        "[Console]::Write((ConvertTo-Json -InputObject @($args) -Compress))\r\n",
    )
    .unwrap();
    let arguments = [
        "",
        "two words",
        "\"",
        "a'b",
        "a\nb\rc",
        "中文🙂",
        "%USERPROFILE% !PATH!",
        "$(New-Item injected.txt) & | ; ` \\",
        "-NamedArgument",
    ]
    .map(String::from);
    let output = run(request(&script, &arguments, root.path(), Transport::Pipe)).await;
    let actual: Vec<String> = serde_json::from_str(&output)
        .unwrap_or_else(|error| panic!("invalid script JSON: {error}: {output:?}"));
    assert_eq!(actual, arguments);
    let mut builder = ChildProcessBuilder::clean_cli(&script);
    builder.args(&arguments).current_dir(root.path());
    let child = builder.output().await.expect("raw stdio PS1 owner starts");
    assert!(
        child.status.success(),
        "raw stdio stderr: {}",
        String::from_utf8_lossy(&child.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Vec<String>>(&child.stdout).unwrap(),
        arguments
    );
    assert!(!root.path().join("injected.txt").exists());
}
