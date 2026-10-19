use std::{path::PathBuf,time::{Duration,Instant}};
#[cfg(target_os="macos")]
use std::process::{Command,ExitStatus,Stdio};
use anyhow::{Result,ensure,Context};
#[cfg(any(target_os="macos",target_os="linux",test))]
use std::path::Path;
use tauri::Manager;

#[cfg(target_os="linux")]
fn default_gui_backend(gdk:Option<&std::ffi::OsStr>,display:Option<&std::ffi::OsStr>,wayland:Option<&std::ffi::OsStr>)->Option<&'static str>{
    if gdk.is_none()&&display.is_some_and(|value|!value.is_empty())&&wayland.is_some_and(|value|!value.is_empty()){Some("x11")}else{None}
}

/// Select the verified XWayland path for GNOME/KDE Wayland sessions. Preserve
/// an explicit user backend and do not affect device-service CLI execution.
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
    let mut child=command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    let deadline=Instant::now()+Duration::from_secs(5);
    loop{if let Some(status)=child.try_wait()?{return Ok(status);}if Instant::now()>=deadline{let _=child.kill();let _=child.wait();anyhow::bail!("系统登录登记命令超时");}std::thread::sleep(Duration::from_millis(25));}
}

fn login_executable()->Result<PathBuf>{
    // AppImage current_exe points into a temporary mount which will disappear.
    #[cfg(target_os="linux")]if let Some(image)=std::env::var_os("APPIMAGE"){
        let path=PathBuf::from(image);ensure!(path.is_absolute()&&path.is_file(),"AppImage 登录启动需要有效的 APPIMAGE 绝对路径");return Ok(path.canonicalize()?);
    }
    let path=std::env::current_exe()?;
    #[cfg(windows)]{Ok(path)}
    #[cfg(not(windows))]{Ok(path.canonicalize()?)}
}

/// The desktop caller disables its reconnect loop before invoking this helper.
/// Only the owner removes its descriptor, after serial teardown completes.
pub fn stop_service(unregister:bool)->Result<()>{
    // The GUI may own a microphone even when the USB daemon failed or crashed.
    // Hold this exclusion until owner teardown/registration cleanup completes.
    let desktop=nomifun_keyboard::desktop_control::request_desktop_shutdown()?;
    stop_owner_with_proof(unregister,Some(&desktop))
}

/// Application-owned update/exit has already awaited shutdown_main. It must
/// not call its own external GUI endpoint and wait for itself to exit.
pub fn stop_owner(unregister:bool)->Result<()>{
    stop_owner_with_proof(unregister,None)
}
fn stop_owner_with_proof(unregister:bool,desktop:Option<&nomifun_keyboard::desktop_control::DesktopQuiescence>)->Result<()>{
    let descriptor=nomifun_keyboard::service::runtime_dir()?.join("owner.json");
    if descriptor.exists(){
        if let Some(proof)=desktop{nomifun_keyboard::service::request_shutdown_after_desktop_exit(proof)?;}
        else{nomifun_keyboard::service::request_shutdown()?;}
        let deadline=Instant::now()+Duration::from_secs(8);
        while descriptor.exists(){ensure!(Instant::now()<deadline,"键盘设备服务未在期限内释放串口；未确认停止");std::thread::sleep(Duration::from_millis(40));}
    }
    if unregister{
        login_registration(false)?;
        #[cfg(target_os="macos")]{let target=format!("gui/{}/com.nomifun.desktop.keyboard",unsafe{libc::getuid()});if command_status(Command::new("/bin/launchctl").args(["print",&target]))?.success(){ensure!(command_status(Command::new("/bin/launchctl").args(["bootout",&target]))?.success(),"无法移除键盘 LaunchAgent");}let path=PathBuf::from(std::env::var_os("HOME").context("HOME unavailable")?).join("Library/LaunchAgents/com.nomifun.desktop.keyboard.plist");if path.exists(){std::fs::remove_file(path)?;}}
    }
    Ok(())
}

#[cfg(any(target_os="linux",test))]
fn desktop_exec(path:&Path)->Result<String>{
    let path=path.to_str().context("登录启动路径不是 UTF-8")?;ensure!(!path.contains(['\n','\r','\0']),"登录启动路径无效");
    // Both Desktop Entry value escaping and Exec quoted-argument escaping apply.
    Ok(path.replace('\\',"\\\\\\\\").replace('"',"\\\\\"").replace('`',"\\\\`").replace('$',"\\\\$").replace('%',"%%"))
}

pub fn login_registration(enabled:bool)->Result<()> {
    let exe=login_executable()?;
    #[cfg(windows)] {
        use std::os::windows::ffi::OsStrExt;
        use windows::{core::w,Win32::{Foundation::{ERROR_FILE_NOT_FOUND,ERROR_PATH_NOT_FOUND},System::Registry::{HKEY_CURRENT_USER,REG_SZ,RegSetKeyValueW,RegDeleteKeyValueW}}};
        let key=w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");let name=w!("NomiFunKeyboard");
        if enabled{
            let mut value=vec![b'"' as u16];value.extend(exe.as_os_str().encode_wide());value.extend("\" --keyboard-service\0".encode_utf16());
            unsafe{RegSetKeyValueW(HKEY_CURRENT_USER,key,name,REG_SZ.0,Some(value.as_ptr().cast()),(value.len()*2).try_into()?).ok()?;}
        }else{let result=unsafe{RegDeleteKeyValueW(HKEY_CURRENT_USER,key,name)};if result!=ERROR_FILE_NOT_FOUND&&result!=ERROR_PATH_NOT_FOUND{result.ok()?;}}
    }
    #[cfg(target_os="macos")] {
        let home=std::env::var_os("HOME").map(PathBuf::from).context("HOME unavailable")?;
        let dir=home.join("Library/LaunchAgents");std::fs::create_dir_all(&dir)?;
        let path=dir.join("com.nomifun.desktop.keyboard.plist");
        let target=format!("gui/{}",unsafe{libc::getuid()});
        let service=format!("{target}/com.nomifun.desktop.keyboard");
        let loaded=command_status(Command::new("/bin/launchctl").args(["print",&service]))?.success();
        if enabled {
            let contents=exe.parent().and_then(Path::parent).context("登录启动需要安装后的 NomiFun.app")?;
            let bundle=contents.parent().filter(|p|p.extension().is_some_and(|x|x=="app")).context("登录启动需要安装后的 NomiFun.app")?;
            ensure!(contents.file_name().is_some_and(|v|v=="Contents")&&!bundle.to_string_lossy().contains("/AppTranslocation/")&&!bundle.starts_with("/Volumes"),"请先将 NomiFun.app 安装到本机固定位置，再启用登录服务");
            let info=plist::Value::from_file(contents.join("Info.plist"))?;
            ensure!(info.as_dictionary().and_then(|v|v.get("CFBundleIdentifier")).and_then(plist::Value::as_string)==Some("com.nomifun.desktop"),"登录服务应用标识不匹配");
            ensure!(command_status(Command::new("/usr/bin/codesign").args(["--verify","--strict","--all-architectures"]).arg(bundle))?.success(),"应用签名验证失败，未登记键盘服务");
            let mut values=plist::Dictionary::new();values.insert("Label".into(),"com.nomifun.desktop.keyboard".into());
            values.insert("ProgramArguments".into(),plist::Value::Array(vec![plist::Value::String(exe.to_string_lossy().into_owned()),"--keyboard-service".into()]));
            values.insert("RunAtLoad".into(),true.into());values.insert("LimitLoadToSessionType".into(),"Aqua".into());
            values.insert("ProcessType".into(),"Interactive".into());values.insert("AssociatedBundleIdentifiers".into(),plist::Value::Array(vec!["com.nomifun.desktop".into()]));
            let mut bytes=Vec::new();plist::Value::Dictionary(values).to_writer_xml(&mut bytes)?;
            let changed=std::fs::read(&path).ok().as_deref()!=Some(bytes.as_slice());
            // Preference changes never terminate a live owner. A changed path
            // is loaded now only after update/exit has released that owner;
            // otherwise the new on-disk path takes effect at the next login.
            let reload=loaded&&changed&&!nomifun_keyboard::service::runtime_dir()?.join("owner.json").exists();
            if reload{ensure!(command_status(Command::new("/bin/launchctl").args(["bootout",&service]))?.success(),"无法卸载旧键盘 LaunchAgent");}
            let temporary=path.with_extension("plist.tmp");std::fs::write(&temporary,bytes)?;std::fs::rename(temporary,&path)?;
            ensure!(command_status(Command::new("/bin/launchctl").args(["enable",&service]))?.success(),"无法启用键盘 LaunchAgent");
            if !loaded||reload{ensure!(command_status(Command::new("/bin/launchctl").args(["bootstrap",&target]).arg(&path))?.success(),"无法加载当前图形会话的键盘 LaunchAgent");}
        } else {
            // A login preference change must not terminate a live recording.
            if loaded||path.exists(){ensure!(command_status(Command::new("/bin/launchctl").args(["disable",&service]))?.success(),"无法禁用键盘 LaunchAgent");}
            // Keep the loaded job's immutable executable identity while it is
            // running. Full unregister removes the plist after owner teardown.
        }
    }
    #[cfg(target_os="linux")] {
        let base=std::env::var_os("XDG_CONFIG_HOME").filter(|p|Path::new(p).is_absolute()).map(PathBuf::from).unwrap_or(std::env::var_os("HOME").map(PathBuf::from).context("HOME unavailable")?.join(".config"));
        let dir=base.join("autostart");std::fs::create_dir_all(&dir)?;let path=dir.join("nomifun-keyboard.desktop");
        if enabled {let escaped=desktop_exec(&exe)?;
            std::fs::write(path,format!("[Desktop Entry]\nType=Application\nName=NomiFun Keyboard\nExec=\"{escaped}\" --keyboard-service\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"))?;
        } else if path.exists(){std::fs::remove_file(path)?;}
    }
    Ok(())
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
}
