//! Best-effort removal of the retired macOS keyboard LaunchAgent only.
//! No legacy IPC, lifetime lock or status is consulted. Callers cannot turn
//! parsing, command or permission errors into application admission failures.
//! It runs only in independent normal-startup work, never uninstall or update.
use std::{path::Path, io::Read};
#[cfg(target_os="macos")]
use std::path::PathBuf;
use anyhow::{Context, Result, ensure};
#[cfg(target_os="macos")]
use std::{process::{Command, Output, Stdio}, time::{Duration, Instant}};

/// Drain both pipes without a reader thread so a stuck command cannot leave a
/// worker behind. The system command and its output have independent bounds.
#[cfg(target_os="macos")]
fn command_output_until(command:&mut Command, deadline: Instant)->Result<Output>{
    ensure!(Instant::now() < deadline, "旧登录项可选清理已到时限");
    use std::os::fd::AsRawFd;
    fn nonblocking(fd:std::os::fd::RawFd)->Result<()>{
        let flags=unsafe{libc::fcntl(fd,libc::F_GETFL)};
        ensure!(flags>=0,"无法读取系统登录命令管道状态");
        ensure!(unsafe{libc::fcntl(fd,libc::F_SETFL,flags|libc::O_NONBLOCK)}>=0,"无法设置系统登录命令管道状态");
        Ok(())
    }
    fn drain(pipe:&mut impl Read,bytes:&mut Vec<u8>)->Result<bool>{
        let mut buffer=[0u8;4096];
        loop{match pipe.read(&mut buffer){
            Ok(0)=>return Ok(true),
            Ok(length)=>{ensure!(bytes.len()+length<=256*1024,"系统登录登记输出过大");bytes.extend_from_slice(&buffer[..length]);},
            Err(error) if error.kind()==std::io::ErrorKind::WouldBlock=>return Ok(false),
            Err(error) if error.kind()==std::io::ErrorKind::Interrupted=>{},
            Err(error)=>return Err(error.into()),
        }}
    }
    let mut child=command.env("LC_ALL","C").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let result=(||{
        let mut stdout=child.stdout.take().context("系统登录命令 stdout 不可用")?;
        let mut stderr=child.stderr.take().context("系统登录命令 stderr 不可用")?;
        nonblocking(stdout.as_raw_fd())?;nonblocking(stderr.as_raw_fd())?;
        let(mut out,mut err)=(Vec::new(),Vec::new());
        loop{
            let out_done=drain(&mut stdout,&mut out)?;let err_done=drain(&mut stderr,&mut err)?;
            if let Some(status)=child.try_wait()?{if out_done&&err_done{return Ok(Output{status,stdout:out,stderr:err});}}
            ensure!(Instant::now()<deadline,"系统登录登记查询超时");
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    if result.is_err(){let _=child.kill();let _=child.wait();}
    result
}

#[cfg(all(target_os="macos", test))]
fn command_output(command: &mut Command) -> Result<Output> {
    command_output_until(command, Instant::now() + Duration::from_secs(5))
}

fn registration_executable_matches(installed:&Path,candidate:&str)->Result<bool>{
    let path=Path::new(candidate);
    if !path.is_absolute()||candidate.contains(['\n','\r','\0']){return Ok(false);}
    let resolved=match path.canonicalize(){Ok(path)=>path,Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(false),Err(error)=>return Err(error.into())};
    Ok(resolved==installed.canonicalize()?)
}

fn registration_arguments_match(installed:&Path,arguments:&[String])->Result<bool>{
    if arguments.len()!=2||arguments[1]!="--keyboard-service"{return Ok(false);}
    registration_executable_matches(installed,&arguments[0])
}


fn read_legacy_registration(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 128 * 1024,
                "旧键盘登录登记不是受限普通文件");
            #[cfg(windows)] {
                use std::os::windows::fs::MetadataExt;
                ensure!(metadata.file_attributes() & 0x400 == 0, "旧键盘登录登记是重解析点");
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let mut options = std::fs::OpenOptions::new(); options.read(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    ensure!(file.metadata()?.is_file(), "旧键盘登录登记不是普通文件");
    let mut contents = Vec::new(); file.take(128 * 1024 + 1).read_to_end(&mut contents)?;
    ensure!(contents.len() <= 128 * 1024, "旧键盘登录登记过大");
    Ok(Some(contents))
}
fn remove_unchanged_registration(path: &Path, contents: &[u8]) -> Result<()> {
    ensure!(read_legacy_registration(path)?.as_deref() == Some(contents), "旧键盘登录登记已变化，未移除");
    std::fs::remove_file(path)?;
    Ok(())
}

trait LoadedRegistration {
    fn query(&mut self) -> Result<Option<String>>;
    fn bootout(&mut self) -> Result<()>;
}
fn cleanup_loaded_owned(exe: &Path, system: &mut impl LoadedRegistration) -> Result<()> {
    let Some(text) = system.query()? else { return Ok(()); };
    let identity = legacy_launchctl_registration(&text).context("旧键盘 LaunchAgent 归属不明，保留未处理")?;
    if !registration_executable_matches(exe, &identity.0)? || !registration_arguments_match(exe, &identity.1)? {
        return Ok(());
    }
    let current = system.query()?.context("旧键盘 LaunchAgent 在清理前变化，保留未处理")?;
    let current = legacy_launchctl_registration(&current).context("旧键盘 LaunchAgent 清理前归属不明")?;
    ensure!(current == identity && registration_executable_matches(exe, &current.0)?
        && registration_arguments_match(exe, &current.1)?, "旧键盘 LaunchAgent 清理前归属变化");
    system.bootout()
}

#[cfg(target_os="macos")]
struct Launchctl { service: String, deadline: Instant }
#[cfg(target_os="macos")]
impl LoadedRegistration for Launchctl {
    fn query(&mut self) -> Result<Option<String>> {
        let output = command_output_until(Command::new("/bin/launchctl").args(["print", &self.service]), self.deadline)?;
        if output.status.success() { return Ok(Some(String::from_utf8(output.stdout)?)); }
        let error = std::str::from_utf8(&output.stderr)?;
        ensure!(error.contains("Could not find service") && error.contains("com.nomifun.desktop.keyboard"),
            "旧键盘 LaunchAgent 查询失败，保留未处理");
        Ok(None)
    }
    fn bootout(&mut self) -> Result<()> {
        ensure!(command_output_until(Command::new("/bin/launchctl").args(["bootout", &self.service]), self.deadline)?.status.success(),
            "旧键盘 LaunchAgent 清理失败，保留未处理");
        Ok(())
    }
}
/// Each independently owned legacy artifact gets its own best-effort attempt.
/// A failure to query a job cannot hide a safe disk cleanup, or vice versa.
fn cleanup_independent(loaded: impl FnOnce() -> Result<()>, disk: impl FnOnce() -> Result<()>) -> Result<()> {
    let mut errors = Vec::new();
    for result in [loaded(), disk()] {
        if let Err(error) = result { errors.push(format!("{error:#}")); }
    }
    ensure!(errors.is_empty(), "{}", errors.join("; "));
    Ok(())
}

#[cfg(target_os="macos")]
fn cleanup_disk_owned(exe: &Path, path: &Path) -> Result<()> {
    let Some(bytes) = read_legacy_registration(path)? else { return Ok(()); };
    let (program, args) = legacy_plist_registration(&bytes).context("旧键盘 plist 格式或归属无效，保留未处理")?;
    if registration_arguments_match(exe, &args)?
        && match program { Some(program) => registration_executable_matches(exe, &program)?, None => true } {
        remove_unchanged_registration(path, &bytes)?;
    }
    Ok(())
}
#[cfg(target_os="macos")]
fn cleanup() -> Result<()> {
    let exe = std::env::current_exe()?.canonicalize()?;
    cleanup_independent(
        || cleanup_loaded_owned(&exe, &mut Launchctl {
            service: format!("gui/{}/com.nomifun.desktop.keyboard", unsafe { libc::getuid() }),
            deadline: Instant::now() + Duration::from_secs(5),
        }),
        || {
            let path = PathBuf::from(std::env::var_os("HOME").context("HOME unavailable")?)
                .join("Library/LaunchAgents/com.nomifun.desktop.keyboard.plist");
            cleanup_disk_owned(&exe, &path)
        },
    )
}

/// Advisory only: modern app/voice/update/uninstall do not consume this result.
#[cfg(target_os="macos")]
fn run_advisory() {
    if let Err(error) = cleanup() {
        tracing::warn!(%error, "retired keyboard LaunchAgent cleanup was skipped; application maintenance is unaffected");
        eprintln!("NomiFun: optional old keyboard login cleanup skipped: {error:#}");
    }
}
/// Schedule only after the modern endpoint and voice initialization dispatch.
#[cfg(target_os="macos")]
pub(super) fn schedule() {
    if let Err(error) = std::thread::Builder::new().name("nomifun-legacy-login-cleanup".into()).spawn(run_advisory) {
        tracing::warn!(%error, "could not schedule optional retired login cleanup");
    }
}

#[cfg(any(target_os="macos",test))]
fn legacy_launchctl_registration(contents:&str)->Option<(String,Vec<String>)>{
    if contents.contains(['\0','\r']){return None;}
    let mut lines=contents.lines();if !lines.next()?.trim().ends_with(" = {"){return None;}
    let(mut depth,mut closed)=(1usize,false);let(mut program,mut arguments)=(None,None);
    while let Some(line)=lines.next(){
        let line=line.trim_start();
        if depth==1{if let Some(value)=line.strip_prefix("program = "){if program.is_some()||value.is_empty(){return None;}program=Some(value.to_owned());continue;}
            if line.trim_end()=="arguments = {"{
                if arguments.is_some(){return None;}let mut values=Vec::new();let mut args_closed=false;
                for argument in lines.by_ref(){let argument=argument.trim_start();if argument.trim_end()=="}"{args_closed=true;break;}if argument.trim_end().is_empty(){return None;}values.push(argument.to_owned());}
                if !args_closed{return None;}arguments=Some(values);continue;
            }
        }
        if line.trim_end()=="}"{depth=depth.checked_sub(1)?;if depth==0{closed=true;break;}}
        // launchctl uses `=>` for event-trigger maps as well as `=` for
        // ordinary sections. Both open a block whose closing brace counts.
        else if line.trim_end().ends_with(" = {")||line.trim_end().ends_with(" => {"){depth+=1;}
    }
    if !closed||lines.any(|line|!line.trim().is_empty()){return None;}
    Some((program?,arguments?))
}

#[cfg(target_os="macos")]
fn legacy_plist_registration(bytes:&[u8])->Result<(Option<String>,Vec<String>)>{
    use serde::de::{Deserialize,Deserializer,MapAccess,Visitor};
    struct Registration{program:Option<String>,arguments:Vec<String>}
    impl<'de> Deserialize<'de> for Registration{
        fn deserialize<D:Deserializer<'de>>(deserializer:D)->std::result::Result<Self,D::Error>{
            struct RegistrationVisitor;
            impl<'de> Visitor<'de> for RegistrationVisitor{
                type Value=Registration;
                fn expecting(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.write_str("unique-key keyboard LaunchAgent dictionary")}
                fn visit_map<M:MapAccess<'de>>(self,mut map:M)->std::result::Result<Registration,M::Error>{
                    let mut keys=std::collections::BTreeSet::new();let(mut label,mut program,mut arguments)=(None,None,None);
                    while let Some(key)=map.next_key::<String>()?{
                        if !keys.insert(key.clone()){return Err(serde::de::Error::custom("duplicate LaunchAgent key"));}
                        match key.as_str(){"Label"=>label=Some(map.next_value::<String>()?),"Program"=>program=Some(map.next_value::<String>()?),"ProgramArguments"=>arguments=Some(map.next_value::<Vec<String>>()?),_=>{map.next_value::<serde::de::IgnoredAny>()?;}}
                    }
                    if label.as_deref()!=Some("com.nomifun.desktop.keyboard"){return Err(serde::de::Error::custom("unrelated LaunchAgent label"));}
                    Ok(Registration{program,arguments:arguments.ok_or_else(||serde::de::Error::custom("missing LaunchAgent arguments"))?})
                }
            }
            deserializer.deserialize_map(RegistrationVisitor)
        }
    }
    let registration:Registration=plist::from_bytes(bytes)?;
    Ok((registration.program,registration.arguments))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FixtureSystem { replies: std::collections::VecDeque<Result<Option<String>>>, bootouts: usize }
    impl LoadedRegistration for FixtureSystem {
        fn query(&mut self) -> Result<Option<String>> { self.replies.pop_front().expect("unexpected legacy query") }
        fn bootout(&mut self) -> Result<()> { self.bootouts += 1; Ok(()) }
    }
    fn loaded(exe: &Path) -> String {
        format!("gui/fixture/com.nomifun.desktop.keyboard = {{\n program = {}\n arguments = {{\n {}\n --keyboard-service\n }}\n}}\n", exe.display(), exe.display())
    }
    #[test]
    fn advisory_plan_never_unloads_missing_malformed_or_other_installation_jobs() {
        let root = tempfile::tempdir().unwrap();
        let exe = root.path().join("Nomi Fun"); std::fs::write(&exe, b"fixture executable, never launched").unwrap();
        let other = root.path().join("other-app"); std::fs::write(&other, b"unrelated").unwrap();
        for reply in [Ok(None), Ok(Some("malformed loaded registration".into())), Ok(Some(loaded(&other))), Err(anyhow::anyhow!("fixture command timeout"))] {
            let mut system = FixtureSystem { replies: [reply].into(), bootouts: 0 };
            let _ = cleanup_loaded_owned(&exe, &mut system);
            assert_eq!(system.bootouts, 0);
        }
    }
    #[test]
    fn advisory_plan_rechecks_exact_identity_before_unloading() {
        let root = tempfile::tempdir().unwrap(); let exe = root.path().join("Nomi Fun");
        std::fs::write(&exe, b"fixture executable, never launched").unwrap();
        let registration = loaded(&exe);
        let mut owned = FixtureSystem { replies: [Ok(Some(registration.clone())), Ok(Some(registration.clone()))].into(), bootouts: 0 };
        cleanup_loaded_owned(&exe, &mut owned).unwrap(); assert_eq!(owned.bootouts, 1);
        let mut changed = FixtureSystem { replies: [Ok(Some(registration)), Ok(None)].into(), bootouts: 0 };
        assert!(cleanup_loaded_owned(&exe, &mut changed).is_err()); assert_eq!(changed.bootouts, 0);
    }
    #[test]
    fn advisory_file_cleanup_retains_changed_and_non_regular_entries() {
        let root = tempfile::tempdir().unwrap(); let path = root.path().join("owned.plist");
        std::fs::write(&path, b"new registration").unwrap();
        assert!(remove_unchanged_registration(&path, b"old registration").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"new registration");
        remove_unchanged_registration(&path, b"new registration").unwrap();
        std::fs::create_dir(&path).unwrap(); assert!(read_legacy_registration(&path).is_err());
    }
    #[test]
    fn loaded_query_failure_does_not_skip_an_independently_owned_disk_entry() {
        let root = tempfile::tempdir().unwrap(); let exe = root.path().join("Nomi Fun");
        std::fs::write(&exe, b"fixture executable, never launched").unwrap();
        let path = root.path().join("owned.plist"); std::fs::write(&path, b"verified owned fixture").unwrap();
        let mut system = FixtureSystem { replies: [Err(anyhow::anyhow!("fixture query denied"))].into(), bootouts: 0 };
        let result = cleanup_independent(
            || cleanup_loaded_owned(&exe, &mut system),
            || remove_unchanged_registration(&path, b"verified owned fixture"),
        );
        assert!(format!("{:#}", result.unwrap_err()).contains("fixture query denied"));
        assert!(!path.exists()); assert_eq!(system.bootouts, 0);
    }
    #[test]
    fn unreadable_disk_entry_does_not_skip_an_independently_owned_loaded_job() {
        let root = tempfile::tempdir().unwrap(); let exe = root.path().join("Nomi Fun");
        std::fs::write(&exe, b"fixture executable, never launched").unwrap();
        let path = root.path().join("owned.plist"); std::fs::create_dir(&path).unwrap();
        let text = loaded(&exe);
        let mut system = FixtureSystem { replies: [Ok(Some(text.clone())), Ok(Some(text))].into(), bootouts: 0 };
        let result = cleanup_independent(
            || cleanup_loaded_owned(&exe, &mut system),
            || read_legacy_registration(&path).map(|_| ()),
        );
        assert!(result.is_err()); assert_eq!(system.bootouts, 1); assert!(path.is_dir());
    }
    #[test]fn legacy_launchctl_requires_unique_closed_top_level_identity(){
        let output="gui/501/com.nomifun.desktop.keyboard = {\n\tprogram = /Applications/Nomi Fun.app/Contents/MacOS/NomiFun\n\targuments = {\n\t\t/Applications/Nomi Fun.app/Contents/MacOS/NomiFun\n\t\t--keyboard-service\n\t}\n\tenvironment = {\n\t\tprogram = /other/app\n\t}\n}\n";
        let expected=("/Applications/Nomi Fun.app/Contents/MacOS/NomiFun".to_owned(),vec!["/Applications/Nomi Fun.app/Contents/MacOS/NomiFun".to_owned(),"--keyboard-service".to_owned()]);
        assert_eq!(legacy_launchctl_registration(output),Some(expected));
        let trailing=output.replace("/Applications/Nomi Fun.app/Contents/MacOS/NomiFun\n","/Applications/Nomi Fun.app/Contents/MacOS/NomiFun \n");
        let parsed=legacy_launchctl_registration(&trailing).unwrap();
        assert!(parsed.0.ends_with(' '));assert!(parsed.1[0].ends_with(' '));
        assert_eq!(legacy_launchctl_registration(&output.replace("NomiFun\n","NomiFun\r\n")),None);
        assert_eq!(legacy_launchctl_registration(&output.replacen("\targuments = {","\tprogram = /other/app\n\targuments = {",1)),None);
        assert_eq!(legacy_launchctl_registration(output.trim_end().trim_end_matches('}')),None);
        assert_eq!(legacy_launchctl_registration("job = {\n environment = {\n program = /other/app\n arguments = {\n /other/app\n --keyboard-service\n }\n }\n}\n"),None);
    }

    // Sanitized native `launchctl print` output from an old keyboard login job.
    const LEGACY_LAUNCHCTL_PRINT:&str=include_str!("fixtures/legacy-launchctl-print.txt");
    #[test]fn legacy_launchctl_accepts_native_event_triggers_and_following_sections(){
        assert_eq!(legacy_launchctl_registration(LEGACY_LAUNCHCTL_PRINT),Some((
            "/Applications/Nomi Fun.app/Contents/MacOS/NomiFun".to_owned(),
            vec!["/Applications/Nomi Fun.app/Contents/MacOS/NomiFun".to_owned(),"--keyboard-service".to_owned()],
        )));
    }
    #[test]fn legacy_launchctl_does_not_read_identity_from_arrow_nested_blocks(){
        let nested=LEGACY_LAUNCHCTL_PRINT.replace("\t\t\tdescriptor = {",concat!(
            "\t\t\tprogram = /other/app\n",
            "\t\t\targuments = {\n\t\t\t\t/other/app\n\t\t\t\t--keyboard-service\n\t\t\t}\n",
            "\t\t\tdescriptor = {",
        ));
        let expected=Some(("/Applications/Nomi Fun.app/Contents/MacOS/NomiFun".to_owned(),
            vec!["/Applications/Nomi Fun.app/Contents/MacOS/NomiFun".to_owned(),"--keyboard-service".to_owned()]));
        assert_eq!(legacy_launchctl_registration(&nested),expected);
        let without_top_level=nested.replace(concat!(
            "\tprogram = /Applications/Nomi Fun.app/Contents/MacOS/NomiFun\n",
            "\targuments = {\n\t\t/Applications/Nomi Fun.app/Contents/MacOS/NomiFun\n\t\t--keyboard-service\n\t}\n",
        ),"");
        assert_eq!(legacy_launchctl_registration(&without_top_level),None);
    }
    #[test]fn legacy_launchctl_rejects_malformed_or_ambiguous_native_output(){
        for (case,invalid) in [
            ("unclosed trigger",LEGACY_LAUNCHCTL_PRINT.replace("\t\t}\n\t}\n\n\tevent channels", "\t}\n\n\tevent channels")),
            ("extra closing brace",format!("{LEGACY_LAUNCHCTL_PRINT}}}\n")),
            ("trailing output",format!("{LEGACY_LAUNCHCTL_PRINT}unexpected trailing output\n")),
            ("invalid root",LEGACY_LAUNCHCTL_PRINT.replacen(" = {", " => {",1)),
            ("duplicate arguments",LEGACY_LAUNCHCTL_PRINT.replacen("\tevent triggers = {", "\targuments = {\n\t\t/other/app\n\t\t--keyboard-service\n\t}\n\tevent triggers = {",1)),
            ("missing program",LEGACY_LAUNCHCTL_PRINT.replace("\tprogram = /Applications/Nomi Fun.app/Contents/MacOS/NomiFun\n", "")),
            ("NUL",LEGACY_LAUNCHCTL_PRINT.replace("state = not running", "state = not\0running")),
        ]{assert_eq!(legacy_launchctl_registration(&invalid),None,"{case}");}
    }

    #[cfg(target_os="macos")]
    #[test]fn legacy_plist_rejects_duplicate_identity_and_extra_arguments(){
        let prefix=r#"<?xml version="1.0"?><plist version="1.0"><dict><key>Label</key><string>com.nomifun.desktop.keyboard</string>"#;
        let identity="<key>Program</key><string>/current/NomiFun</string><key>ProgramArguments</key><array><string>/current/NomiFun</string><string>--keyboard-service</string></array>";
        let bytes=format!("{prefix}{identity}</dict></plist>");
        assert_eq!(legacy_plist_registration(bytes.as_bytes()).unwrap(),(Some("/current/NomiFun".to_owned()),vec!["/current/NomiFun".to_owned(),"--keyboard-service".to_owned()]));
        for duplicate in ["<key>Program</key><string>/other/NomiFun</string>","<key>ProgramArguments</key><array><string>/other/NomiFun</string></array>","<key>Label</key><string>other</string>"]{
            let invalid=format!("{prefix}{identity}{duplicate}</dict></plist>");assert!(legacy_plist_registration(invalid.as_bytes()).is_err());
        }
        let dir=tempfile::tempdir().unwrap();let installed=dir.path().join("NomiFun");std::fs::write(&installed,b"current").unwrap();
        let arguments=vec![installed.to_string_lossy().into_owned(),"--keyboard-service".to_owned()];
        assert!(registration_arguments_match(&installed,&arguments).unwrap());
        let mut extra=arguments;extra.push("--extra".into());assert!(!registration_arguments_match(&installed,&extra).unwrap());
    }
    #[cfg(target_os="macos")]
    #[test]fn legacy_command_capture_bounds_output_and_waits_for_exit(){
        let output=command_output(Command::new("/usr/bin/printf").arg("keyboard-test")).unwrap();
        assert!(output.status.success());assert_eq!(output.stdout,b"keyboard-test");
        assert!(command_output(Command::new("/usr/bin/head").args(["-c","300000","/dev/zero"])).is_err());
        let start=Instant::now();assert!(command_output(Command::new("/bin/sleep").arg("8")).is_err());
        assert!(start.elapsed()<Duration::from_secs(7));
    }

}
