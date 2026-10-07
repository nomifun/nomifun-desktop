//! Adopt CEF's documented application event protocol on the host's own
//! NSApplication subclass, preserving its existing sendEvent implementation.
//! No system NSEvent method, WebKit private API, or global input is modified.
use std::{cell::Cell, sync::{OnceLock, Weak}};
use objc2::{ClassType, Encode, MainThreadMarker, runtime::{AnyClass, AnyObject, AnyProtocol, Bool, ProtocolBuilder, Sel}, sel};
use objc2_app_kit::{NSApplication, NSEvent};
use crate::engine::Engine;

type SendEvent = unsafe extern "C" fn(&AnyObject, Sel, &NSEvent);
type Imp = unsafe extern "C" fn();
static ORIGINAL: OnceLock<SendEvent> = OnceLock::new();
static OWNER: OnceLock<Weak<Engine>> = OnceLock::new();
thread_local! { static SENDING: Cell<bool> = const { Cell::new(false) }; }

#[link(name = "objc")]
unsafe extern "C" {
    fn class_addMethod(class: *const AnyClass, selector: Sel, implementation: Imp, types: *const std::ffi::c_char) -> Bool;
    fn class_addProtocol(class: *const AnyClass, protocol: *const AnyProtocol) -> Bool;
    fn class_getMethodImplementation(class: *const AnyClass, selector: Sel) -> Imp;
    fn class_replaceMethod(class: *const AnyClass, selector: Sel, implementation: Imp, types: *const std::ffi::c_char) -> Option<Imp>;
}

extern "C" fn is_sending(_: &AnyObject, _: Sel) -> Bool { Bool::new(SENDING.get()) }
extern "C" fn set_sending(_: &AnyObject, _: Sel, value: Bool) { SENDING.set(value.as_bool()); }
unsafe extern "C" fn send_event(app: &AnyObject, selector: Sel, event: &NSEvent) {
    if OWNER.get().and_then(Weak::upgrade).is_some_and(|engine| engine.blocks_user_event(event)) { return; }
    let previous = SENDING.replace(true);
    // Invoke the saved implementation directly. The class is unchanged, so
    // Tao's own super dispatch and Cmd-key-up handling remain intact.
    if let Some(original) = ORIGINAL.get() { unsafe { original(app, selector, event) }; }
    SENDING.set(previous);
}

pub(crate) fn install(owner: Weak<Engine>) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("CEF application bridge requires the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    let class = app.class();
    if class == NSApplication::class() { return Err("CEF requires an application-owned NSApplication subclass".into()); }
    if OWNER.get().is_some() { return Err("CEF application bridge is already installed".into()); }
    // cef_application_mac.h declares these protocols in the embedder, rather
    // than requiring the framework dylib to export their runtime objects.
    let readable = AnyProtocol::get(c"CrAppProtocol").unwrap_or_else(|| {
        let mut p = ProtocolBuilder::new(c"CrAppProtocol").expect("new CEF read protocol");
        p.add_method_description::<(), Bool>(sel!(isHandlingSendEvent), true);
        p.register()
    });
    let writable = AnyProtocol::get(c"CrAppControlProtocol").unwrap_or_else(|| {
        let mut p = ProtocolBuilder::new(c"CrAppControlProtocol").expect("new CEF control protocol");
        p.add_protocol(readable);
        p.add_method_description::<(Bool,), ()>(sel!(setHandlingSendEvent:), true);
        p.register()
    });
    let protocol = AnyProtocol::get(c"CefAppProtocol").unwrap_or_else(|| {
        let mut p = ProtocolBuilder::new(c"CefAppProtocol").expect("new CEF app protocol");
        p.add_protocol(writable);
        p.register()
    });
    let bool_encoding = Bool::ENCODING.to_string();
    let getter = std::ffi::CString::new(format!("{bool_encoding}@:")).unwrap();
    let setter = std::ffi::CString::new(format!("v@:{bool_encoding}")).unwrap();
    unsafe {
        let original = class_getMethodImplementation(class, sel!(sendEvent:));
        if !class_addMethod(class, sel!(isHandlingSendEvent), std::mem::transmute(is_sending as extern "C" fn(_, _) -> _), getter.as_ptr()).as_bool()
            || !class_addMethod(class, sel!(setHandlingSendEvent:), std::mem::transmute(set_sending as extern "C" fn(_, _, _)), setter.as_ptr()).as_bool() {
            return Err("The application already has an incompatible CEF event bridge".into());
        }
        if !class_addProtocol(class, protocol).as_bool() { return Err("CEF application protocol registration failed".into()); }
        ORIGINAL.set(std::mem::transmute::<Imp, SendEvent>(original)).map_err(|_| "CEF sendEvent bridge is already installed")?;
        OWNER.set(owner).map_err(|_| "CEF event owner is already installed")?;
        class_replaceMethod(class, sel!(sendEvent:), std::mem::transmute(send_event as SendEvent), c"v@:@".as_ptr());
    }
    Ok(())
}
