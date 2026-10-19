use super::*;
use nomi_process_runtime::FrozenOutput;

struct Fixture {
    runner:Runner,
    directory:tempfile::TempDir,
    target:Value,
}

impl Fixture {
    fn new()->Self {
        let runner=Runner::new();
        let directory=tempfile::tempdir().unwrap();
        let mut target=json!({"invocation_id":Uuid::now_v7().to_string(),"call_id":Uuid::now_v7().to_string()});
        let mut start=target.clone();
        start["op"]=json!("start");
        start["cwd"]=json!(directory.path());
        start["expire_on_idle"]=json!(false);
        start["output_limit_bytes"]=json!(5);
        start["command"]=if cfg!(windows) {
            json!({"kind":"program","program":"powershell.exe","args":["-NoProfile","-NonInteractive","-Command",
                "[Console]::ReadLine() | Out-Null; [Console]::Write('archive-proof')"]})
        } else {
            json!({"kind":"program","program":"/bin/sh","args":["-c","IFS= read -r line; printf 'archive-proof'"]})
        };
        let started=runner.call(&serde_json::to_vec(&start).unwrap());
        assert_eq!(started["ok"],true,"{started}");
        target["session_id"]=started["result"]["session_id"].clone();
        Self {runner,directory,target}
    }
    fn request(&self,operation:&str)->Value {
        let mut request=self.target.clone();request["op"]=json!(operation);request
    }
    fn call(&self,request:Value)->Value {self.runner.call(&serde_json::to_vec(&request).unwrap())}
    fn finish(&self)->Value {
        assert_eq!(self.call(self.request("close_stdin"))["ok"],true);
        let mut request=self.request("terminal");request["include_output_replay"]=json!(true);
        let until=Instant::now()+Duration::from_secs(15);
        loop {
            let response=self.call(request.clone());
            assert_eq!(response["ok"],true,"{response}");
            if !response["result"]["terminal"].is_null() {return response["result"]["terminal"].clone();}
            assert!(Instant::now()<until,"original native owner never froze its terminal");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {let _=self.runner.call(br#"{"op":"shutdown"}"#);}
}

#[test]
fn native_output_archive_waits_for_original_eof_and_rejects_stale_exact_owners() {
    let fixture=Fixture::new();
    let request=fixture.request("output_archive");
    let waiting=fixture.call(request.clone());
    assert_eq!(waiting["ok"],true,"{waiting}");
    assert!(waiting["result"].get("output_replay").unwrap().is_null());
    let mut foreign=request.clone();foreign["invocation_id"]=json!(Uuid::now_v7().to_string());
    let response=fixture.call(foreign);
    assert_eq!(response["ok"],false,"{response}");
    assert_eq!(response["error"]["code"],"owner_mismatch");
    let mut stale=request.clone();stale["session_id"]=json!(SessionId::new().to_string());
    let response=fixture.call(stale);
    assert_eq!(response["ok"],false,"{response}");
    assert_eq!(response["error"]["code"],"session_not_found");
    // Failed observations do not retire the real owner or close its input.
    let terminal=fixture.finish();
    assert_eq!(terminal["state"],"exited");
    assert_eq!(terminal["cleanup"]["reaped"],true);
    let archived=fixture.call(request);
    assert_eq!(archived["ok"],true,"{archived}");
    let replay:FrozenOutput=serde_json::from_value(archived["result"]["output_replay"].clone()).unwrap();
    replay.validate().unwrap();
    assert_eq!(replay.snapshot_from(OutputCursor::START).text(),"proof");
    assert_eq!(replay.snapshot_from(OutputCursor::START).raw_bytes(),b"proof");
    assert_eq!(replay.snapshot_from(OutputCursor::START).dropped_bytes,8);
    assert_eq!(replay.snapshot_from(OutputCursor::START).next_cursor.offset(),13);
    let inline:FrozenOutput=serde_json::from_value(terminal["output_replay"].clone()).unwrap();
    for cursor in 0..=15 {
        assert_eq!(inline.snapshot_from(OutputCursor::new(cursor)),replay.snapshot_from(OutputCursor::new(cursor)));
    }
    let persisted=serde_json::to_vec(&replay).unwrap();
    let path=fixture.directory.path().to_owned();
    assert_eq!(fixture.runner.call(br#"{"op":"shutdown"}"#)["result"]["exact"],true);
    assert!(path.is_dir(),"readonly archive never disposes the user workspace");
    drop(fixture);
    drop(replay);
    drop(inline);
    let restored:FrozenOutput=serde_json::from_slice(&persisted).unwrap();
    assert_eq!(restored.snapshot_from(OutputCursor::new(10)).raw_bytes(),b"oof");
}

#[test]
fn native_output_archive_is_additive_and_never_changes_legacy_terminal_measurements() {
    let fixture=Fixture::new();
    let terminal=fixture.finish();
    let legacy=fixture.call(fixture.request("terminal"));
    assert!(legacy["result"]["terminal"].get("output_replay").is_none());
    let mut without_archive=terminal.clone();without_archive.as_object_mut().unwrap().remove("output_replay");
    assert_eq!(legacy["result"]["terminal"],without_archive);
    let mut poll=fixture.request("poll");poll["cursor"]=json!(10);poll["include_output_replay"]=json!(true);
    let poll=fixture.call(poll);
    assert_eq!(poll["ok"],true,"{poll}");
    let replay:FrozenOutput=serde_json::from_value(poll["result"]["output_replay"].clone()).unwrap();
    assert_eq!(replay.snapshot_from(OutputCursor::new(10)).text(),"oof");
    let legacy_text=poll["result"]["output"]["chunks"].as_array().unwrap().iter()
        .map(|chunk|chunk["text"].as_str().unwrap()).collect::<String>();
    assert_eq!(legacy_text,replay.snapshot_from(OutputCursor::new(10)).text());
    let mut cancel=fixture.request("cancel");cancel["include_output_replay"]=json!(true);
    let cancel=fixture.call(cancel);
    assert_eq!(cancel["result"]["state"],"exited","observing archived terminal does not recancel natural completion");
    assert_eq!(cancel["result"]["output_replay"],terminal["output_replay"]);
}

#[test]
fn native_terminal_subscription_reports_natural_exit_without_poll_and_does_not_block_exact_control() {
    let fixture=Fixture::new();
    let mut wait=fixture.request("wait_terminal");wait["observation_ms"]=json!(30_000);wait["include_output_replay"]=json!(true);
    let mut foreign=wait.clone();foreign["call_id"]=json!(Uuid::now_v7().to_string());
    assert_eq!(fixture.call(foreign)["error"]["code"],"owner_mismatch");
    std::thread::scope(|threads| {
        let (entered,ready)=std::sync::mpsc::sync_channel(1);let (finished,completion)=std::sync::mpsc::sync_channel(1);
        let original=&fixture;
        threads.spawn(move || {entered.send(()).unwrap();finished.send(original.call(wait)).unwrap();});
        ready.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(completion.try_recv().is_err(),"a running process does not produce a synthetic terminal");
        let start=Instant::now();
        // This ordinary original native operation must pass the concurrent
        // pure subscription. It closes stdin; it does not cancel a sibling or
        // poll to discover that the child exited.
        assert_eq!(fixture.call(fixture.request("close_stdin"))["ok"],true);
        assert!(start.elapsed()<Duration::from_secs(3));
        let response=completion.recv_timeout(Duration::from_secs(15)).unwrap();assert_eq!(response["ok"],true,"{response}");
        let terminal=&response["result"]["terminal"];assert_eq!(terminal["state"],"exited");assert_eq!(terminal["cleanup"]["reaped"],true);
        let replay:FrozenOutput=serde_json::from_value(terminal["output_replay"].clone()).unwrap();
        assert_eq!(replay.snapshot_from(OutputCursor::START).raw_bytes(),b"proof");
    });
}

#[test]
fn native_terminal_observation_budget_only_rearms_listener_and_never_expires_the_process() {
    let fixture=Fixture::new();let mut wait=fixture.request("wait_terminal");wait["observation_ms"]=json!(1);
    let response=fixture.call(wait);assert_eq!(response["ok"],true,"{response}");assert!(response["result"]["terminal"].is_null());
    let response=fixture.call(fixture.request("terminal"));assert!(response["result"]["terminal"].is_null());
    let terminal=fixture.finish();assert_eq!(terminal["state"],"exited");
}

#[test]
fn exact_system_terminal_release_never_kills_live_process_and_original_archive_survives_native_owner_retirement() {
    let fixture=Fixture::new();let response=fixture.call(fixture.request("release_terminal"));
    assert_eq!(response["result"]["released"],false,"{response}");
    let terminal=fixture.finish();let replay=terminal["output_replay"].clone();
    let response=fixture.call(fixture.request("release_terminal"));assert_eq!(response["result"]["released"],true,"{response}");
    assert_eq!(fixture.call(fixture.request("terminal"))["error"]["code"],"session_not_found");
    assert_eq!(fixture.call(fixture.request("release_terminal"))["result"]["released"],true);
    let replay:FrozenOutput=serde_json::from_value(replay).unwrap();assert_eq!(replay.snapshot_from(OutputCursor::START).raw_bytes(),b"proof");
}
