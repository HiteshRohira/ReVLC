//! Places a native video view underneath the (transparent) web view, so
//! libVLC draws the video and Core Animation composites the HTML UI on top.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::{MainThreadMarker, MainThreadOnly, Message, sel};
use objc2::rc::Retained;
use block2::RcBlock;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSCursor, NSEvent, NSEventMask, NSView, NSWindow,
    NSWindowDidEnterFullScreenNotification, NSWindowDidExitFullScreenNotification, NSWindowOrderingMode,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectNSDelayedPerforming};

/// Creates the video view below `webview` and returns it as an `NSView*`.
/// Must run on the main thread. The view lives as long as the window.
///
/// `on_pointer` is called for every mouse move or scroll in the window, from
/// a native event monitor. The UI uses it to show its controls, so they still
/// wake up if libVLC's view takes the event before the web view sees it.
pub fn attach(
    webview: *mut c_void,
    window: *mut c_void,
    on_pointer: impl Fn() + 'static,
) -> Result<*mut c_void, String> {
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

    window.setAcceptsMouseMovedEvents(true);
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        on_pointer();
        event.as_ptr()
    });
    let mask = NSEventMask::MouseMoved | NSEventMask::LeftMouseDragged | NSEventMask::ScrollWheel;
    // The monitor lives for the whole app.
    std::mem::forget(unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) });

    // After entering or leaving fullscreen, the web page can lose focus
    // while the web view stays first responder, and the shortcuts stop
    // working. Once the transition has settled, resign and take it again,
    // which refocuses the page.
    let (web, win) = (webview.retain(), window.retain());
    let refocus = RcBlock::new(move |_: NonNull<NSNotification>| unsafe {
        win.performSelector_withObject_afterDelay(sel!(makeFirstResponder:), None, 0.3);
        win.performSelector_withObject_afterDelay(sel!(makeFirstResponder:), Some(&web), 0.35);
    });
    let center = NSNotificationCenter::defaultCenter();
    for name in unsafe { [NSWindowDidEnterFullScreenNotification, NSWindowDidExitFullScreenNotification] } {
        // The observers live for the whole app.
        std::mem::forget(unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), Some(window), None, &refocus) });
    }

    Ok(Retained::into_raw(video) as *mut c_void)
}

/// Hides the cursor until the mouse moves (used while video plays).
pub fn hide_cursor() {
    NSCursor::setHiddenUntilMouseMoves(true);
}
