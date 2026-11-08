use std::{path::PathBuf,time::Duration};
use anyhow::{Result,ensure,Context};
use tauri::{Emitter, Manager};

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

/// A fixed in-app route; subtitle input never supplies a URL or route.
pub fn open_keyboard_settings(app: &tauri::AppHandle) -> Result<()> {
    let window = app.get_webview_window("main").context("主窗口尚未就绪")?;
    window.unminimize()?;
    window.show()?;
    window.set_focus()?;
    app.emit_to("main", "companion-navigate", "/settings/voice-keyboard")?;
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
}
