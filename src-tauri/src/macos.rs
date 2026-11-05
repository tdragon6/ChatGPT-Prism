use crate::{Host, request_manager_close};
use objc2::{
    MainThreadMarker, ffi, msg_send,
    rc::Retained,
    runtime::{AnyObject, Imp, Sel},
    sel,
};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSApplication, NSApplicationTerminateReply, NSWindow,
};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager};
use std::{
    mem,
    sync::{OnceLock, atomic::Ordering},
};
use tauri::{AppHandle, Manager, WebviewWindow};

static APP: OnceLock<AppHandle> = OnceLock::new();

pub fn set_manager_appearance(window: &WebviewWindow, appearance: &str) -> tauri::Result<()> {
    let appearance = appearance.to_owned();
    window.with_webview(move |webview| {
        // Match the native title bar without changing the app-wide system appearance.
        let window = unsafe { &*webview.ns_window().cast::<NSWindow>() };
        let name = unsafe {
            match appearance.as_str() {
                "dark" => Some(NSAppearanceNameDarkAqua),
                "light" => Some(NSAppearanceNameAqua),
                _ => None,
            }
        };
        let appearance = name.and_then(NSAppearance::appearanceNamed);
        window.setAppearance(appearance.as_deref());
    })
}

fn system_quit() -> bool {
    let manager = NSAppleEventManager::sharedAppleEventManager();
    let Some(event) = manager.currentAppleEvent() else {
        return false;
    };
    let reason: Option<Retained<NSAppleEventDescriptor>> = unsafe {
        // CoreServices kAEQuitReason is a 32-bit Apple event keyword.
        msg_send![&*event, paramDescriptorForKeyword: u32::from_be_bytes(*b"why?")]
    };
    reason.is_some_and(|reason| {
        [reason.enumCodeValue(), reason.typeCodeValue()]
            .into_iter()
            .any(|code| matches!(&code.to_be_bytes(), b"quia" | b"shut" | b"rest" | b"rlgo"))
    })
}

extern "C-unwind" fn should_terminate(
    _: &AnyObject,
    _: Sel,
    _: &NSApplication,
) -> NSApplicationTerminateReply {
    let Some(app) = APP.get() else {
        return NSApplicationTerminateReply::TerminateNow;
    };
    if system_quit() || app.state::<Host>().quitting.load(Ordering::Acquire) {
        return NSApplicationTerminateReply::TerminateNow;
    }
    let app = app.clone();
    // Let AppKit cancel the native quit before changing window visibility.
    tauri::async_runtime::spawn(async move {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || request_manager_close(&handle, true));
    });
    NSApplicationTerminateReply::TerminateCancel
}

pub fn install_quit_handler(app: &AppHandle) -> Result<(), &'static str> {
    let mtm = MainThreadMarker::new().ok_or("AppKit setup must run on the main thread")?;
    let application = NSApplication::sharedApplication(mtm);
    let delegate = application
        .delegate()
        .ok_or("Missing AppKit application delegate")?;
    let object: &AnyObject = delegate.as_ref();
    APP.set(app.clone())
        .map_err(|_| "AppKit quit handler already installed")?;
    let installed = unsafe {
        // Tao's delegate omits this optional selector. Preserve its other callbacks.
        let method = mem::transmute::<
            extern "C-unwind" fn(&AnyObject, Sel, &NSApplication) -> NSApplicationTerminateReply,
            Imp,
        >(should_terminate);
        ffi::class_addMethod(
            std::ptr::from_ref(object.class()).cast_mut(),
            sel!(applicationShouldTerminate:),
            method,
            c"Q@:@".as_ptr(),
        )
        .as_bool()
    };
    if !installed {
        return Err("Could not install AppKit quit handler");
    }
    application.setDelegate(None);
    application.setDelegate(Some(&delegate));
    Ok(())
}
