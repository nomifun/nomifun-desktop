use std::{path::{Path,PathBuf},time::Duration};
#[cfg(target_os="macos")]
use std::{io::Read,process::{Command,ExitStatus,Output,Stdio},time::Instant};
use anyhow::{Result,ensure,Context};
use tauri::Manager;

#[cfg(target_os="linux")]
fn default_gui_backend(gdk:Option<&std::ffi::OsStr>,display:Option<&std::ffi::OsStr>,wayland:Option<&std::ffi::OsStr>)->Option<&'static str>{
    if gdk.is_none()&&display.is_some_and(|value|!value.is_empty())&&wayland.is_some_and(|value|!value.is_empty()){Some("x11")}else{None}
}

/// Select the verified XWayland path for GNOME/KDE Wayland sessions. Preserve
/// an explicit user backend.
///
/// # Safety
/// Call before GTK initialization and before creating any worker threads.
#[cfg(target_os="linux")]
pub unsafe fn configure_gui_backend(){
    if let Some(backend)=default_gui_backend(std::env::var_os("GDK_BACKEND").as_deref(),std::env::var_os("DISPLAY").as_deref(),std::env::var_os("WAYLAND_DISPLAY").as_deref()){
        unsafe{std::env::set_var("GDK_BACKEND",backend);}
    }
}

#[cfg(target_os="macos")]
fn command_status(command:&mut Command)->Result<ExitStatus>{
    Ok(command_output(command)?.status)
}

/// Drain both pipes without a reader thread so a stuck command cannot leave a
/// worker behind. The system command and its output have independent bounds.
#[cfg(target_os="macos")]
fn command_output(command:&mut Command)->Result<Output>{
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
        let deadline=Instant::now()+Duration::from_secs(5);
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
pub fn retire_legacy_service()->Result<()>{
    let exe=std::env::current_exe()?.canonicalize()?;
    let runtime=nomifun_keyboard::ipc::runtime_dir()?;
    let _retirement=nomifun_keyboard::maintenance::retire_legacy_service_in(runtime.clone(),Duration::from_secs(30),|observed|{
        nomifun_keyboard::legacy_service::validate_runtime_ownership(&exe,&runtime,observed.desktop)
    })?;
    remove_legacy_login_registration(&login_executable()?)
}

/// Short-lived package/uninstall entrypoint. Keep startup and lifetime locks
/// through login cleanup, including when only an old daemon remains alive.
pub fn stop_application(unregister:bool)->Result<()>{
    let exe=std::env::current_exe()?.canonicalize()?;
    let runtime=nomifun_keyboard::ipc::runtime_dir()?;
    let _maintenance=nomifun_keyboard::maintenance::MaintenanceGuard::prepare_with_intent_checked_in(runtime.clone(),Duration::from_secs(30),&mut nomifun_keyboard::maintenance::ResumeIntent::default(),|observed|{
        nomifun_keyboard::legacy_service::validate_runtime_ownership(&exe,&runtime,observed.desktop)
    })?;
    if unregister{remove_legacy_login_registration(&login_executable()?)?;}
    Ok(())
}

fn registration_executable_matches(installed:&Path,candidate:&str)->Result<bool>{
    let path=Path::new(candidate);
    if !path.is_absolute()||candidate.contains(['\n','\r','\0']){return Ok(false);}
    let resolved=match path.canonicalize(){Ok(path)=>path,Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(false),Err(error)=>return Err(error.into())};
    Ok(resolved==installed.canonicalize()?)
}

#[cfg(target_os="macos")]
fn registration_arguments_match(installed:&Path,arguments:&[String])->Result<bool>{
    if arguments.len()!=2||arguments[1]!="--keyboard-service"{return Ok(false);}
    registration_executable_matches(installed,&arguments[0])
}

#[cfg(any(target_os="macos",target_os="linux"))]
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

#[cfg(any(target_os="macos",target_os="linux"))]
fn remove_unchanged_registration(path:&Path,contents:&[u8])->Result<()>{
    ensure!(read_legacy_registration(path)?.as_deref()==Some(contents),"旧键盘登录登记已变化，未移除");
    std::fs::remove_file(path)?;
    Ok(())
}

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
    #[cfg(target_os="macos")] {
        let path=PathBuf::from(std::env::var_os("HOME").context("HOME unavailable")?).join("Library/LaunchAgents/com.nomifun.desktop.keyboard.plist");
        let contents=read_legacy_registration(&path)?;
        let disk_owned=if let Some(bytes)=&contents{
            match legacy_plist_registration(bytes){
                Ok((program,arguments))=>registration_arguments_match(exe,&arguments)?&&match program{Some(program)=>registration_executable_matches(exe,&program)?,None=>true},
                Err(_)=>false,
            }
        }else{false};
        let service=format!("gui/{}/com.nomifun.desktop.keyboard",unsafe{libc::getuid()});
        let output=command_output(Command::new("/bin/launchctl").args(["print",&service]))?;
        if output.status.success(){
            let text=std::str::from_utf8(&output.stdout).context("旧键盘 LaunchAgent 查询输出不是 UTF-8")?;
            let (program,arguments)=legacy_launchctl_registration(text).context("无法证明已加载键盘 LaunchAgent 的程序归属，未卸载")?;
            if registration_executable_matches(exe,&program)?&&registration_arguments_match(exe,&arguments)?{
                let current=command_output(Command::new("/bin/launchctl").args(["print",&service]))?;
                ensure!(current.status.success(),"旧键盘 LaunchAgent 在清理前变化，未卸载");
                let identity=legacy_launchctl_registration(std::str::from_utf8(&current.stdout)?).context("旧键盘 LaunchAgent 在清理前归属不明，未卸载")?;
                ensure!(identity==(program.clone(),arguments.clone())&&registration_executable_matches(exe,&identity.0)?&&registration_arguments_match(exe,&identity.1)?,"旧键盘 LaunchAgent 在清理前归属变化，未卸载");
                ensure!(command_status(Command::new("/bin/launchctl").args(["bootout",&service]))?.success(),"无法移除已验证的旧键盘 LaunchAgent");
            }
        }else{
            let error=std::str::from_utf8(&output.stderr).context("旧键盘 LaunchAgent 查询错误不是 UTF-8")?;
            ensure!(error.contains("Could not find service")&&error.contains("com.nomifun.desktop.keyboard"),"无法确认旧键盘 LaunchAgent 是否加载，未清理");
        }
        if disk_owned{remove_unchanged_registration(&path,contents.as_deref().context("旧键盘 plist 不可用")?)?;}
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
        else if line.trim_end().ends_with(" = {"){depth+=1;}
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

#[cfg(any(windows,target_os="macos",target_os="linux"))]
fn on_main(window:&tauri::WebviewWindow,run:impl FnOnce(tauri::WebviewWindow)->Result<()>+Send+'static)->Result<()>{
    let copy=window.clone();let(tx,rx)=std::sync::mpsc::sync_channel(1);
    let expired=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));let pending=expired.clone();
    window.run_on_main_thread(move||{if pending.load(std::sync::atomic::Ordering::Acquire){return;}let _=tx.send(run(copy).map_err(|e|e.to_string()));})?;
    let result=rx.recv_timeout(Duration::from_secs(3));
    if result.is_err(){expired.store(true,std::sync::atomic::Ordering::Release);}
    result.context("原生窗口线程未响应")?.map_err(anyhow::Error::msg)
}

pub fn show_without_activation(window:&tauri::WebviewWindow)->Result<()> {
    #[cfg(windows)] {
        // Keep subtitle visibility and z-order on the same native path.
        // Its automatic appearance must never request foreground activation;
        // ordinary mouse activation stays available for selection and buttons.
        on_main(window,|window|{
            use windows::Win32::UI::WindowsAndMessaging::{ShowWindow,IsWindowVisible,SW_SHOWNOACTIVATE};
            let hwnd=window.hwnd()?;
            unsafe{let _=ShowWindow(hwnd,SW_SHOWNOACTIVATE);ensure!(IsWindowVisible(hwnd).as_bool(),"字幕窗口未能显示");}
            Ok(())
        })?;
    }
    #[cfg(target_os="macos")] {
        // AppKit orderFrontRegardless changes ordering without making the
        // subtitle window key or activating NSApplication.
        on_main(window,|window|{
            let pointer=window.ns_window()?;ensure!(!pointer.is_null(),"字幕窗口不可用");
            unsafe{let native=&*(pointer as *const objc2_app_kit::NSWindow);native.setHidesOnDeactivate(false);native.orderFrontRegardless();}
            Ok(())
        })?;
    }
    #[cfg(target_os="linux")] {
        use gtk::prelude::*;
        // GTK sets the X11 user-time hint / Wayland activation policy when
        // mapped. Selection still works when the user explicitly clicks.
        on_main(window,|window|{
            let gtk_window=window.gtk_window()?;
            if gtk_window.is_visible(){return Ok(());}
            gtk_window.set_focus_on_map(false);gtk_window.set_skip_taskbar_hint(true);gtk_window.set_type_hint(gtk::gdk::WindowTypeHint::Utility);
            const KEY:&str="nomifun-subtitle-map-focus-v1";
            if unsafe{gtk_window.data::<bool>(KEY)}.is_none(){
                // Keep the initial WM map non-focusable, then allow ordinary
                // user clicks after MapNotify. Changing this only during the
                // first click loses that click's focus grant on KWin Wayland.
                gtk_window.connect_map_event(|window,_|{window.set_accept_focus(true);gtk::glib::Propagation::Proceed});
                unsafe{gtk_window.set_data(KEY,true);}
            }
            gtk_window.set_accept_focus(false);
            gtk_window.realize();
            // GTK can retain the application's earlier user/startup timestamp.
            // EWMH user-time zero explicitly marks an automatic X11 map as
            // non-activating. A later real click remains eligible for focus.
            let x11=gtk_window.window().and_then(|native|native.downcast::<gdkx11::X11Window>().ok());
            if let Some(native)=&x11{native.set_user_time(0);}
            // The window starts hidden, so Wry also keeps its WebKit child
            // hidden. Map the whole hierarchy, as Tao's normal show() does.
            gtk_window.show_all();
            if let Some(native)=x11{native.set_user_time(0);}
            Ok(())
        })?;
    }
    Ok(())
}

/// Confirm native visibility on the UI thread before discarding a document.
/// Tao's private VISIBLE cache is not the authority for this utility window.
pub fn hide_subtitle(window:&tauri::WebviewWindow)->Result<()> {
    #[cfg(windows)]{
        on_main(window,|window|{
            use windows::Win32::UI::WindowsAndMessaging::{ShowWindow,IsWindowVisible,SW_HIDE};
            let hwnd=window.hwnd()?;
            unsafe{let _=ShowWindow(hwnd,SW_HIDE);ensure!(!IsWindowVisible(hwnd).as_bool(),"字幕窗口未能隐藏");}
            Ok(())
        })?;
    }
    #[cfg(target_os="macos")]{
        on_main(window,|window|{
            let pointer=window.ns_window()?;ensure!(!pointer.is_null(),"字幕窗口不可用");
            unsafe{let native=&*(pointer as *const objc2_app_kit::NSWindow);native.orderOut(None);ensure!(!native.isVisible(),"字幕窗口未能隐藏");}
            Ok(())
        })?;
    }
    #[cfg(target_os="linux")]{
        use gtk::prelude::*;
        on_main(window,|window|{
            let native=window.gtk_window()?;
            native.hide();ensure!(!native.is_visible(),"字幕窗口未能隐藏");
            Ok(())
        })?;
    }
    Ok(())
}

pub fn set_subtitle_topmost(window:&tauri::WebviewWindow,topmost:bool)->Result<()> {
    #[cfg(windows)]{
        on_main(window,move|window|{
            use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos,HWND_TOPMOST,HWND_NOTOPMOST,SWP_NOMOVE,SWP_NOSIZE,SWP_NOACTIVATE,SWP_NOOWNERZORDER};
            unsafe{SetWindowPos(window.hwnd()?,Some(if topmost{HWND_TOPMOST}else{HWND_NOTOPMOST}),0,0,0,0,SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE|SWP_NOOWNERZORDER)?;}
            Ok(())
        })?;
    }
    #[cfg(not(windows))]window.set_always_on_top(topmost)?;
    Ok(())
}

pub fn toggle_main(app:&tauri::AppHandle)->Result<()> {
    let window=app.get_webview_window("main").context("主窗口尚未就绪")?;
    #[cfg(windows)]
    let foreground={
        use windows::Win32::UI::WindowsAndMessaging::{GetAncestor,GetForegroundWindow,GA_ROOT};
        let main=window.hwnd()?;
        // WebView2 transfers keyboard focus to its child HWND. Tao's cached
        // focused bit may then be false while this top-level window remains
        // foreground; using that bit makes every K1 press show it again.
        unsafe{let active=GetForegroundWindow();active==main||GetAncestor(active,GA_ROOT)==main}
    };
    #[cfg(not(windows))]
    let foreground=window.is_focused()?;
    if window.is_visible()?&&foreground{window.hide()?;}
    else {
        window.unminimize()?;
        #[cfg(windows)]{window.show()?;window.set_focus()?;}
        #[cfg(target_os="macos")] {
            on_main(&window,|window|{
                let mtm=objc2::MainThreadMarker::new().context("AppKit 主线程不可用")?;
                let pointer=window.ns_window()?;ensure!(!pointer.is_null(),"主窗口不可用");
                unsafe{(&*(pointer as *const objc2_app_kit::NSWindow)).makeKeyAndOrderFront(None);}
                #[allow(deprecated)]objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
                Ok(())
            })?;
        }
        #[cfg(target_os="linux")]{use gtk::prelude::*;on_main(&window,|window|{let native=window.gtk_window()?;native.set_accept_focus(true);native.set_focus_on_map(true);native.present();Ok(())})?;}
    }
    Ok(())
}

pub fn position_file(root:&std::path::Path)->PathBuf{root.join("keyboard-window-position.json")}

pub const SUBTITLE_WIDTH:f64=720.0;
pub const SUBTITLE_HEIGHT:f64=112.0;

pub fn position_subtitle(window:&tauri::WebviewWindow,initial:bool)->Result<()> {
    if let Some(monitor)=window.current_monitor()?.or(window.primary_monitor()?){
        let area=monitor.work_area();
        // GTK has not mapped a newly built hidden window yet. Its native
        // allocation can still be 1px, rather than the requested logical size.
        let requested=tauri::LogicalSize::new(SUBTITLE_WIDTH,SUBTITLE_HEIGHT).to_physical::<u32>(monitor.scale_factor());
        let size=if initial{requested}else{let native=window.outer_size()?;if native.width<=1||native.height<=1{requested}else{native}};
        let x=area.position.x+area.size.width.saturating_sub(size.width) as i32/2;
        let margin=(40.0*monitor.scale_factor()).round() as i32;
        let y=area.position.y+(area.size.height.saturating_sub(size.height) as i32-margin).max(0);
        window.set_position(tauri::PhysicalPosition::new(x,y))?;
    }
    Ok(())
}

#[cfg(test)]mod tests{use super::*;
    #[cfg(target_os="linux")]
    #[test]fn default_backend_preserves_explicit_choice_and_requires_both_displays(){
        use std::ffi::OsStr;
        assert_eq!(default_gui_backend(None,Some(OsStr::new(":0")),Some(OsStr::new("wayland-0"))),Some("x11"));
        assert_eq!(default_gui_backend(Some(OsStr::new("wayland")),Some(OsStr::new(":0")),Some(OsStr::new("wayland-0"))),None);
        assert_eq!(default_gui_backend(None,None,Some(OsStr::new("wayland-0"))),None);
        assert_eq!(default_gui_backend(None,Some(OsStr::new(":0")),None),None);
        assert_eq!(default_gui_backend(None,Some(OsStr::new("")),Some(OsStr::new("wayland-0"))),None);
    }
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
    #[cfg(any(target_os="macos",target_os="linux"))]
    #[test]fn legacy_registration_cleanup_refuses_symlinks_and_changed_files(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("registration");
        std::fs::write(&path,b"old").unwrap();assert_eq!(read_legacy_registration(&path).unwrap(),Some(b"old".to_vec()));
        std::fs::write(&path,b"new").unwrap();assert!(remove_unchanged_registration(&path,b"old").is_err());
        assert_eq!(std::fs::read(&path).unwrap(),b"new");
        let link=dir.path().join("symlink");std::os::unix::fs::symlink(&path,&link).unwrap();
        assert!(read_legacy_registration(&link).is_err());assert_eq!(std::fs::read(&path).unwrap(),b"new");
        remove_unchanged_registration(&path,b"new").unwrap();assert!(!path.exists());
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
