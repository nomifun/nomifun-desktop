use std::time::Duration;
use anyhow::Result;
#[cfg(any(not(target_os="macos"),test))]
use std::path::{Path,PathBuf};
#[cfg(any(not(target_os="macos"),test))]
use anyhow::ensure;
#[cfg(any(target_os="linux",test))]
use anyhow::Context;

#[cfg(not(target_os="macos"))]
fn login_executable()->Result<PathBuf>{
    // AppImage login entries name the persistent image; native process identity
    // is checked separately against current_exe, never against this wrapper.
    #[cfg(target_os="linux")]if let Some(image)=std::env::var_os("APPIMAGE"){
        let path=PathBuf::from(image);ensure!(path.is_absolute()&&path.is_file(),"AppImage 登录启动需要有效的 APPIMAGE 绝对路径");return Ok(path.canonicalize()?);
    }
    Ok(std::env::current_exe()?.canonicalize()?)
}

/// Retire an installed daemon before registering this GUI's desktop endpoint.
/// The retirement guard shares startup admission with the current GUI and
/// holds the legacy owner lock until its matching login entries are removed.
#[cfg(not(target_os="macos"))]
pub fn retire_legacy_service()->Result<()>{
    let exe=std::env::current_exe()?.canonicalize()?;
    let runtime=nomifun_desktop_maintenance::ipc::runtime_dir()?;
    let _retirement=nomifun_desktop_maintenance::maintenance::retire_legacy_service_in(runtime.clone(),Duration::from_secs(30),|observed|{
        nomifun_desktop_maintenance::legacy_service::validate_runtime_ownership(&exe,&runtime,observed.desktop)
    })?;
    remove_legacy_login_registration(&login_executable()?)
}

/// Short-lived package/uninstall entrypoint. Keep startup and lifetime locks
/// through login cleanup, including when only an old daemon remains alive.
#[cfg(not(target_os="macos"))]
pub fn stop_application(unregister:bool)->Result<()>{
    let exe=std::env::current_exe()?.canonicalize()?;
    let runtime=nomifun_desktop_maintenance::ipc::runtime_dir()?;
    let _maintenance=nomifun_desktop_maintenance::maintenance::MaintenanceGuard::prepare_with_intent_checked_in(runtime.clone(),Duration::from_secs(30),&mut nomifun_desktop_maintenance::maintenance::ResumeIntent::default(),|observed|{
        nomifun_desktop_maintenance::legacy_service::validate_runtime_ownership(&exe,&runtime,observed.desktop)
    })?;
    if unregister{remove_legacy_login_registration(&login_executable()?)?;}
    Ok(())
}

/// Both macOS maintenance command aliases concern only the current app.
/// Old login cleanup belongs exclusively to independent normal-startup work.
#[cfg(target_os="macos")]
pub fn stop_application(_unregister: bool) -> Result<()> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let runtime = nomifun_desktop_maintenance::ipc::runtime_dir()?;
    let _maintenance = nomifun_desktop_maintenance::maintenance::MaintenanceGuard::prepare_application_with_intent_checked_in(
        runtime.clone(), Duration::from_secs(30), &mut nomifun_desktop_maintenance::maintenance::ResumeIntent::default(),
        |observed| nomifun_desktop_maintenance::desktop_control::validate_runtime_ownership(&exe, &runtime, observed.desktop),
    )?;
    Ok(())
}

#[cfg(any(not(target_os="macos"),test))]
fn registration_executable_matches(installed:&Path,candidate:&str)->Result<bool>{
    let path=Path::new(candidate);
    if !path.is_absolute()||candidate.contains(['\n','\r','\0']){return Ok(false);}
    let resolved=match path.canonicalize(){Ok(path)=>path,Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(false),Err(error)=>return Err(error.into())};
    Ok(resolved==installed.canonicalize()?)
}

#[cfg(target_os="linux")]
fn read_legacy_registration(path:&Path)->Result<Option<Vec<u8>>>{
    use std::io::Read;
    match std::fs::symlink_metadata(path){
        Ok(metadata)=>ensure!(metadata.is_file()&&!metadata.file_type().is_symlink()&&metadata.len()<=128*1024,"旧键盘登录登记不是受限普通文件"),
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(None),
        Err(error)=>return Err(error.into()),
    }
    let mut options=std::fs::OpenOptions::new();options.read(true);
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK);
    let file=options.open(path)?;ensure!(file.metadata()?.is_file(),"旧键盘登录登记不是普通文件");
    let mut contents=Vec::new();file.take(128*1024+1).read_to_end(&mut contents)?;
    ensure!(contents.len()<=128*1024,"旧键盘登录登记过大");
    Ok(Some(contents))
}

#[cfg(target_os="linux")]
fn remove_unchanged_registration(path:&Path,contents:&[u8])->Result<()>{
    ensure!(read_legacy_registration(path)?.as_deref()==Some(contents),"旧键盘登录登记已变化，未移除");
    std::fs::remove_file(path)?;
    Ok(())
}

#[cfg(not(target_os="macos"))]
fn remove_legacy_login_registration(exe:&Path)->Result<()>{
    #[cfg(windows)]{
        use windows::{core::w,Win32::{Foundation::{ERROR_FILE_NOT_FOUND,ERROR_PATH_NOT_FOUND},System::Registry::{HKEY_CURRENT_USER,RRF_RT_REG_SZ,RegGetValueW,RegDeleteKeyValueW}}};
        fn read_run()->Result<Option<String>>{
            let mut size=0;
            let result=unsafe{RegGetValueW(HKEY_CURRENT_USER,w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),w!("NomiFunKeyboard"),RRF_RT_REG_SZ,None,None,Some(&mut size))};
            if result==ERROR_FILE_NOT_FOUND||result==ERROR_PATH_NOT_FOUND{return Ok(None);}
            result.ok()?;ensure!((2..=128*1024).contains(&size)&&size%2==0,"旧键盘 Run 登记大小无效");
            let mut value=vec![0u16;(size/2)as usize];
            unsafe{RegGetValueW(HKEY_CURRENT_USER,w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),w!("NomiFunKeyboard"),RRF_RT_REG_SZ,None,Some(value.as_mut_ptr().cast()),Some(&mut size))}.ok()?;
            ensure!(size%2==0&&size>=2&&(size/2)as usize<=value.len(),"旧键盘 Run 登记发生变化");value.truncate((size/2)as usize);
            ensure!(value.last()==Some(&0)&&!value[..value.len()-1].contains(&0),"旧键盘 Run 登记含无效终止符");
            Ok(Some(String::from_utf16(&value[..value.len()-1])?))
        }
        if let Some(command)=read_run()?{if let Some(candidate)=legacy_windows_executable(&command){if registration_executable_matches(exe,candidate)?{
            ensure!(read_run()?.as_deref()==Some(command.as_str()),"旧键盘 Run 登记已变化，未移除");
            unsafe{RegDeleteKeyValueW(HKEY_CURRENT_USER,w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),w!("NomiFunKeyboard"))}.ok()?;
        }}}
    }
    #[cfg(target_os="linux")] {
        let base=std::env::var_os("XDG_CONFIG_HOME").filter(|p|Path::new(p).is_absolute()).map(PathBuf::from).unwrap_or(std::env::var_os("HOME").map(PathBuf::from).context("HOME unavailable")?.join(".config"));
        let path=base.join("autostart/nomifun-keyboard.desktop");
        if let Some(contents)=read_legacy_registration(&path)?{if let Ok(text)=std::str::from_utf8(&contents){if let Some(candidate)=legacy_desktop_executable(text){if registration_executable_matches(exe,&candidate)?{remove_unchanged_registration(&path,&contents)?;}}}}
    }
    Ok(())
}

#[cfg(any(target_os="linux",test))]
fn desktop_exec(path:&Path)->Result<String>{
    let path=path.to_str().context("登录启动路径不是 UTF-8")?;ensure!(!path.contains(['\n','\r','\0']),"登录启动路径无效");
    Ok(path.replace('\\',"\\\\\\\\").replace('"',"\\\\\"").replace('`',"\\\\`").replace('$',"\\\\$").replace('%',"%%"))
}

// Parse only registrations written by the retired keyboard service. Ambiguous
// values are retained instead of being interpreted as shell commands.
#[cfg(any(windows,test))]
fn legacy_windows_executable(command:&str)->Option<&str>{
    let candidate=command.strip_prefix('"')?.strip_suffix("\" --keyboard-service")?;
    if candidate.is_empty()||candidate.contains(['"','\n','\r','\0']){return None;}
    Some(candidate)
}

#[cfg(any(target_os="linux",test))]
fn legacy_desktop_executable(contents:&str)->Option<String>{
    if contents.contains('\0'){return None;}
    let(mut active,mut seen)=(false,false);let(mut command,mut entry_type)=(None,None);
    for line in contents.lines(){
        let line=line.trim();
        if line.starts_with('['){active=line=="[Desktop Entry]";if active{if seen{return None;}seen=true;}continue;}
        if active&&!line.starts_with('#'){if let Some((key,value))=line.split_once('='){match key.trim(){
            "Exec"=>{if command.is_some(){return None;}command=Some(value);},
            "Type"=>{if entry_type.is_some(){return None;}entry_type=Some(value);},
            "DBusActivatable"=>return None,
            _=>{},
        }}}
    }
    if entry_type!=Some("Application"){return None;}
    let encoded=command?.strip_prefix('"')?.strip_suffix("\" --keyboard-service")?;
    let mut chars=encoded.chars();let mut candidate=String::new();
    while let Some(ch)=chars.next(){match ch{
        '%' =>{if chars.next()?!='%'{return None;}candidate.push('%');},
        '\\'=>{if chars.next()?!='\\'{return None;}match chars.next()?{'\\'=>{if chars.next()?!='\\'{return None;}candidate.push('\\');},ch @('"'|'$'|'`')=>candidate.push(ch),_=>return None}},
        '"'|'$'|'`'|'\n'|'\r'|'\0'=>return None,
        ch=>candidate.push(ch),
    }}
    if candidate.is_empty()||desktop_exec(Path::new(&candidate)).ok()?.as_str()!=encoded{return None;}
    Some(candidate)
}

#[cfg(test)]mod tests{use super::*;
    #[test]fn desktop_exec_preserves_a_single_argument_and_neutralizes_field_codes(){
        assert_eq!(desktop_exec(Path::new("/opt/Nomi Fun/100%/app")).unwrap(),"/opt/Nomi Fun/100%%/app");
        assert_eq!(desktop_exec(Path::new("/opt/a\"$`\\b")).unwrap(),"/opt/a\\\\\"\\\\$\\\\`\\\\\\\\b");
        assert!(desktop_exec(Path::new("/opt/a\nExec=bad")).is_err());
    }
    #[test]fn legacy_windows_command_requires_the_exact_retired_service_arguments(){
        assert_eq!(legacy_windows_executable("\"C:\\Program Files\\NomiFun\\NomiFun.exe\" --keyboard-service"),Some("C:\\Program Files\\NomiFun\\NomiFun.exe"));
        for value in ["NomiFun.exe --keyboard-service","\"C:\\app.exe\" --keyboard-service --extra","\"C:\\app.exe\" --keyboard-service ","\"C:\\app.exe\" --keyboard-service\0", "\"C:\\a\"b.exe\" --keyboard-service"]{assert_eq!(legacy_windows_executable(value),None,"{value:?}");}
    }
    #[test]fn legacy_desktop_command_decodes_the_old_writer_and_rejects_ambiguity(){
        let path=Path::new("/opt/Nomi Fun/100%/a\"$`\\b");
        let contents=format!("[Desktop Entry]\nType=Application\nExec=\"{}\" --keyboard-service\n",desktop_exec(path).unwrap());
        assert_eq!(legacy_desktop_executable(&contents),Some(path.to_str().unwrap().to_owned()));
        for override_entry in [format!("{contents}DBusActivatable=true\n"),contents.replace("Type=Application","Type=Link"),format!("{contents}Type=Application\n")]{assert_eq!(legacy_desktop_executable(&override_entry),None);}
        for invalid in [format!("{contents}Exec=\"/other/app\" --keyboard-service\n"),format!("{contents}[Desktop Entry]\nExec=\"/other/app\" --keyboard-service\n"),"[Desktop Entry]\nExec=\"/app\" --keyboard-service --extra\n".to_owned(),"[Desktop Entry]\nExec=\"/app%f\" --keyboard-service\n".to_owned(),"[Desktop Action Other]\nExec=\"/app\" --keyboard-service\n".to_owned()]{assert_eq!(legacy_desktop_executable(&invalid),None,"{invalid:?}");}
    }
    #[test]fn legacy_registration_requires_the_actual_current_installation(){
        let dir=tempfile::tempdir().unwrap();let installed=dir.path().join("NomiFun");let other=dir.path().join("other-NomiFun");
        std::fs::write(&installed,b"current").unwrap();std::fs::write(&other,b"other").unwrap();
        assert!(registration_executable_matches(&installed,installed.to_str().unwrap()).unwrap());
        assert!(!registration_executable_matches(&installed,other.to_str().unwrap()).unwrap());
        assert!(!registration_executable_matches(&installed,"NomiFun").unwrap());
        assert!(!registration_executable_matches(&installed,dir.path().join("missing").to_str().unwrap()).unwrap());
        #[cfg(unix)]{
            let alias=dir.path().join("alias");std::os::unix::fs::symlink(&installed,&alias).unwrap();
            assert!(registration_executable_matches(&installed,alias.to_str().unwrap()).unwrap());
        }
    }
    #[cfg(target_os="linux")]
    #[test]fn legacy_registration_cleanup_refuses_symlinks_and_changed_files(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("registration");
        std::fs::write(&path,b"old").unwrap();assert_eq!(read_legacy_registration(&path).unwrap(),Some(b"old".to_vec()));
        std::fs::write(&path,b"new").unwrap();assert!(remove_unchanged_registration(&path,b"old").is_err());
        assert_eq!(std::fs::read(&path).unwrap(),b"new");
        let link=dir.path().join("symlink");std::os::unix::fs::symlink(&path,&link).unwrap();
        assert!(read_legacy_registration(&link).is_err());assert_eq!(std::fs::read(&path).unwrap(),b"new");
        remove_unchanged_registration(&path,b"new").unwrap();assert!(!path.exists());
    }

}
