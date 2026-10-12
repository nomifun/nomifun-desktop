//! Public subclass behavior for our embedded child, never a system-class hook.
//!
//! Like Wry's child WKWebView workaround, menu shortcuts must reach AppKit
//! before WKWebView consumes performKeyEquivalent without performing the edit.
use crate::engine::Page;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::Bool,
};
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType};
use objc2_foundation::{NSObjectProtocol, NSRect};
use objc2_web_kit::{WKWebView, WKWebViewConfiguration};
use std::{
    cell::Cell,
    sync::{Weak, atomic::Ordering},
};

pub(crate) struct ViewState {
    owner: Weak<Page>,
    routing_menu: Cell<bool>,
}
define_class!(
    #[unsafe(super(WKWebView))]
    #[name = "NomiFunWKWebView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ViewState]
    pub(crate) struct BrowserView;

    unsafe impl NSObjectProtocol for BrowserView {}
    impl BrowserView {
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self,event:&NSEvent)->Bool {
            let Some(owner)=self.ivars().owner.upgrade() else{return Bool::NO;};
            if owner.close_requested.load(Ordering::Acquire)||!owner.is_visible(){return Bool::NO;}
            let focused=self.window().and_then(|window|window.firstResponder()).is_some_and(|responder|{
                let is_view:bool=unsafe{msg_send![&responder,isKindOfClass:objc2::class!(NSView)]};
                is_view&&unsafe{msg_send![&responder,isDescendantOf:self]}
            });
            if !focused{return Bool::NO;}
            if owner.input_locked(){return Bool::YES;}
            if self.ivars().routing_menu.get(){return Bool::NO;}
            if is_standard_edit_shortcut(event) {
                self.ivars().routing_menu.set(true);
                let handled=NSApplication::sharedApplication(self.mtm()).mainMenu().is_some_and(|menu|menu.performKeyEquivalent(event));
                self.ivars().routing_menu.set(false);
                return Bool::new(handled);
            }
            unsafe{msg_send![super(self),performKeyEquivalent:event]}
        }
    }
);
impl BrowserView {
    pub(crate) fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        configuration: &WKWebViewConfiguration,
        owner: Weak<Page>,
    ) -> Retained<WKWebView> {
        let this = Self::alloc(mtm).set_ivars(ViewState {
            owner,
            routing_menu: Cell::new(false),
        });
        let view: Retained<Self> =
            unsafe { msg_send![super(this),initWithFrame:frame,configuration:configuration] };
        view.into_super()
    }
}
fn is_standard_edit_shortcut(event: &NSEvent) -> bool {
    if event.r#type() != NSEventType::KeyDown
        || !event
            .modifierFlags()
            .contains(NSEventModifierFlags::Command)
    {
        return false;
    }
    event.charactersIgnoringModifiers().is_some_and(|text| {
        matches!(
            text.to_string().to_ascii_lowercase().as_str(),
            "a" | "c" | "v" | "x" | "z"
        )
    })
}
