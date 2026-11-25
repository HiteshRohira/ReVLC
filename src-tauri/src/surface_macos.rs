//! Places a native video view underneath the (transparent) web view, so
//! libVLC draws the video and Core Animation composites the HTML UI on top.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::{MainThreadMarker, MainThreadOnly, Message, sel};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use block2::RcBlock;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSCursor, NSEvent, NSEventMask, NSView, NSWindow,
    NSWindowDidEnterFullScreenNotification, NSWindowDidExitFullScreenNotification, NSWindowOrderingMode,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectNSDelayedPerforming, NSObjectProtocol};

/// One window's video view and the AppKit hooks made for it. Dropping it
/// (on the main thread, once libVLC has stopped drawing into the view)
/// removes the hooks and releases the view, so a closed window leaves
/// nothing behind.
pub struct Surface {
    video: Retained<NSView>,
    monitor: Option<Retained<AnyObject>>,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

impl Surface {
    /// The video view, as the `NSView*` libVLC draws into.
    pub fn view(&self) -> *mut c_void {
        Retained::as_ptr(&self.video) as *mut c_void
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: the tokens are the ones AppKit returned when they were added.
        unsafe {
            if let Some(monitor) = self.monitor.take() {
                NSEvent::removeMonitor(&monitor);
            }
            let center = NSNotificationCenter::defaultCenter();
            for observer in self.observers.drain(..) {
                // The block it holds (and the window and web view the block holds) go with it.
                center.removeObserver(observer.as_ref());
            }
        }
        self.video.removeFromSuperview();
    }
}

/// Creates the video view below `webview`. Must run on the main thread.
/// Keep the [`Surface`] until the window has closed and its player has
/// stopped, then drop it on the main thread.
///
/// `on_pointer` is called for every mouse move or scroll in the window, from
/// a native event monitor. The UI uses it to show its controls, so they still
/// wake up if libVLC's view takes the event before the web view sees it.
pub fn attach(
    webview: *mut c_void,
    window: *mut c_void,
    on_pointer: impl Fn() + 'static,
) -> Result<Surface, String> {
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
    // The monitor sees every window's events, and each player window has
    // its own: only this window's count.
    let this = window as *const NSWindow;
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit hands the monitor a live event.
        let from = unsafe { event.as_ref() }.window(mtm);
        if from.is_some_and(|w| std::ptr::eq(&*w, this)) {
            on_pointer();
        }
        event.as_ptr()
    });
    let mask = NSEventMask::MouseMoved | NSEventMask::LeftMouseDragged | NSEventMask::ScrollWheel;
    // SAFETY: the handler returns the event it was given, as AppKit requires.
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) };

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
    let observers = unsafe { [NSWindowDidEnterFullScreenNotification, NSWindowDidExitFullScreenNotification] }
        .into_iter()
        .map(|name| unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), Some(window), None, &refocus) })
        .collect();

    Ok(Surface { video, monitor, observers })
}

/// Hides the cursor until the mouse moves (used while video plays).
pub fn hide_cursor() {
    NSCursor::setHiddenUntilMouseMoves(true);
}
