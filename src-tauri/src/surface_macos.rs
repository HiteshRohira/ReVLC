//! Places a native video view underneath the (transparent) web view, so
//! libVLC draws the video and Core Animation composites the HTML UI on top.

use std::ffi::c_void;

use objc2::{MainThreadMarker, MainThreadOnly};
use objc2::rc::Retained;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSColor, NSView, NSWindow, NSWindowOrderingMode};

/// Creates the video view below `webview` and returns it as an `NSView*`.
/// Must run on the main thread. The view lives as long as the window.
pub fn attach(webview: *mut c_void, window: *mut c_void) -> Result<*mut c_void, String> {
    let mtm = MainThreadMarker::new().ok_or("attach must run on the main thread")?;
    // SAFETY: Tauri hands us the window's live WKWebView and NSWindow.
    let webview: &NSView = unsafe { &*(webview as *const NSView) };
    let window: &NSWindow = unsafe { &*(window as *const NSWindow) };
    let parent = unsafe { webview.superview() }.ok_or("the web view has no parent view")?;

    let video = NSView::initWithFrame(NSView::alloc(mtm), parent.bounds());
    video.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    // Layer-backed, so libVLC's OpenGL view respects z-order instead of
    // drawing over every sibling (including the web view).
    video.setWantsLayer(true);
    parent.setWantsLayer(true);
    parent.addSubview_positioned_relativeTo(&video, NSWindowOrderingMode::Below, Some(webview));

    // The window is transparent so the web view can be; paint black behind the video instead.
    window.setBackgroundColor(Some(&NSColor::blackColor()));
    window.setOpaque(true);

    Ok(Retained::into_raw(video) as *mut c_void)
}
